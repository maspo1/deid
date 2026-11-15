//! XGBoost 방식 그래디언트 부스팅 트리 (다중 분류, `multi:softprob`).
//!
//! 외부 라이브러리 없이 XGBoost 의 학습 알고리즘을 그대로 구현한다.
//! * 목적함수: 소프트맥스 교차 엔트로피 — 클래스 k 의 기울기 g = p_k − y_k, 곡률 h = max(2·p_k(1−p_k), ε)
//! * 분할 이득: ½·[G_L²/(H_L+λ) + G_R²/(H_R+λ) − G²/(H+λ)] − γ
//! * 잎 가중치: −G/(H+λ) × η (학습률)
//! * 분할 후보: 특징별 히스토그램(최대 64칸) — XGBoost `tree_method=hist` 와 같은 방식
//!
//! 매 라운드마다 클래스별로 트리를 1개씩 만들고, 예측은 클래스별 트리 점수 합에 소프트맥스를 취한다.

#[derive(Clone, Debug)]
pub struct Node {
    /// 분할 특징 (잎이면 -1)
    pub feat: i32,
    /// x[feat] < thr 이면 왼쪽
    pub thr: f32,
    pub left: u32,
    pub right: u32,
    /// 잎 점수
    pub value: f32,
}

#[derive(Clone, Debug)]
pub struct Tree {
    pub class: usize,
    pub nodes: Vec<Node>,
}

impl Tree {
    fn score(&self, x: &[f32]) -> f32 {
        let mut i = 0usize;
        loop {
            let n = &self.nodes[i];
            if n.feat < 0 {
                return n.value;
            }
            i = if x[n.feat as usize] < n.thr { n.left } else { n.right } as usize;
        }
    }
}

#[derive(Clone, Debug)]
pub struct Model {
    pub n_class: usize,
    pub n_feat: usize,
    pub trees: Vec<Tree>,
    /// 분기 없는 평가용 색인 — 처음 예측할 때 한 번 만듦 (트리를 바꾼 뒤에는 새 Model 을 만들 것)
    pub fast: std::sync::OnceLock<Option<QuickScorer>>,
}

/// 분기 없는 트리 앙상블 평가 (QuickScorer — Lucchese 외, SIGIR 2015).
///
/// 노드마다 "x[f] ≥ thr 이면 왼쪽 서브트리의 잎은 도달 불가" → 트리의 잎 비트마스크에서 그 잎들을 지운다.
/// 특징별로 모든 트리의 임계값을 오름차순으로 모아 두면, x[f] 이하인 임계값까지 **순서대로 AND 만** 하면 된다.
/// 끝나면 트리마다 남은 가장 낮은 비트가 도착 잎 (노드를 하나씩 따라가며 분기할 필요가 없다).
/// 점수 합은 원래와 같은 트리 순서로 더하므로 결과가 비트 단위로 같다.
#[derive(Clone, Debug)]
pub struct QuickScorer {
    /// 특징별, 임계값 오름차순: (임계값, 트리, 남길 잎 마스크)
    thr: Vec<Vec<f32>>,
    tree: Vec<Vec<u32>>,
    mask: Vec<Vec<u64>>,
    /// 트리별 잎 값 (왼쪽부터) · 클래스
    leaves: Vec<Vec<f32>>,
    class: Vec<usize>,
}

impl QuickScorer {
    /// 잎이 64개를 넘는 트리가 있으면 None (그때는 노드를 따라가는 평가)
    fn build(m: &Model) -> Option<QuickScorer> {
        let mut per: Vec<Vec<(f32, u32, u64)>> = vec![Vec::new(); m.n_feat];
        let mut leaves = Vec::with_capacity(m.trees.len());
        // 중위 순회로 잎 번호를 왼쪽부터 매기고, 노드마다 왼쪽 서브트리 잎 범위를 지우는 마스크
        fn walk(t: &Tree, i: usize, ti: u32, lv: &mut Vec<f32>, per: &mut Vec<Vec<(f32, u32, u64)>>) -> Option<(usize, usize)> {
            let n = t.nodes.get(i)?;
            if n.feat < 0 {
                lv.push(n.value);
                return Some((lv.len() - 1, lv.len()));
            }
            let (a, b) = walk(t, n.left as usize, ti, lv, per)?;
            let (_, c) = walk(t, n.right as usize, ti, lv, per)?;
            if c > 64 {
                return None;
            }
            let left: u64 = (if b == 64 { u64::MAX } else { (1u64 << b) - 1 }) & !((1u64 << a) - 1);
            per.get_mut(n.feat as usize)?.push((n.thr, ti, !left));
            Some((a, c))
        }
        for (ti, t) in m.trees.iter().enumerate() {
            let mut lv = Vec::new();
            walk(t, 0, ti as u32, &mut lv, &mut per)?;
            leaves.push(lv);
        }
        for v in &mut per {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        Some(QuickScorer {
            thr: per.iter().map(|v| v.iter().map(|e| e.0).collect()).collect(),
            tree: per.iter().map(|v| v.iter().map(|e| e.1).collect()).collect(),
            mask: per.iter().map(|v| v.iter().map(|e| e.2).collect()).collect(),
            leaves,
            class: m.trees.iter().map(|t| t.class).collect(),
        })
    }

    fn margins(&self, x: &[f32], n_class: usize) -> Vec<f32> {
        let mut v = vec![u64::MAX; self.leaves.len()];
        for (f, thr) in self.thr.iter().enumerate() {
            let xf = x[f];
            let (tree, mask) = (&self.tree[f], &self.mask[f]);
            // 원래 규칙: x < thr 이면 왼쪽. NaN 은 비교가 모두 거짓 → 항상 오른쪽 → 모든 노드 적용
            let k = if xf.is_nan() { thr.len() } else { thr.partition_point(|&t| t <= xf) };
            for j in 0..k {
                v[tree[j] as usize] &= mask[j];
            }
        }
        let mut m = vec![0f32; n_class];
        for (t, lv) in self.leaves.iter().enumerate() {
            m[self.class[t]] += lv[v[t].trailing_zeros() as usize];
        }
        m
    }
}

pub fn softmax(m: &mut [f32]) {
    let mx = m.iter().cloned().fold(f32::MIN, f32::max);
    let mut s = 0.0;
    for v in m.iter_mut() {
        *v = (*v - mx).exp();
        s += *v;
    }
    for v in m.iter_mut() {
        *v /= s;
    }
}

impl Model {
    /// 클래스별 확률 (합 1)
    pub fn predict_proba(&self, x: &[f32]) -> Vec<f32> {
        let mut m = match self.fast.get_or_init(|| QuickScorer::build(self)) {
            Some(q) => q.margins(x, self.n_class),
            None => self.margins_by_nodes(x),
        };
        softmax(&mut m);
        m
    }

    /// 노드를 하나씩 따라가는 평가 (기준 구현)
    fn margins_by_nodes(&self, x: &[f32]) -> Vec<f32> {
        let mut m = vec![0f32; self.n_class];
        for t in &self.trees {
            m[t.class] += t.score(x);
        }
        m
    }

    /// 텍스트 직렬화 (src/rules/ml/model.txt)
    pub fn to_text(&self) -> String {
        let mut s = format!("deid-gbdt v1\nclasses {}\nfeatures {}\ntrees {}\n", self.n_class, self.n_feat, self.trees.len());
        for t in &self.trees {
            s.push_str(&format!("tree {} {}\n", t.class, t.nodes.len()));
            for n in &t.nodes {
                if n.feat < 0 {
                    s.push_str(&format!("L {}\n", n.value));
                } else {
                    s.push_str(&format!("N {} {} {} {}\n", n.feat, n.thr, n.left, n.right));
                }
            }
        }
        s
    }

    pub fn from_text(s: &str) -> Result<Model, String> {
        let mut lines = s.lines().filter(|l| !l.trim().is_empty());
        fn header<'a>(lines: &mut impl Iterator<Item = &'a str>, key: &str) -> Result<usize, String> {
            let l = lines.next().ok_or("모델 파일이 짧음")?;
            let v = l.strip_prefix(key).ok_or_else(|| format!("{key} 없음"))?;
            v.trim().parse().map_err(|_| format!("{key} 값 오류"))
        }
        if lines.next() != Some("deid-gbdt v1") {
            return Err("모델 형식이 아님".into());
        }
        let n_class = header(&mut lines, "classes")?;
        let n_feat = header(&mut lines, "features")?;
        let n_trees = header(&mut lines, "trees")?;
        let mut trees = Vec::with_capacity(n_trees);
        for _ in 0..n_trees {
            let h: Vec<usize> = lines
                .next()
                .and_then(|l| l.strip_prefix("tree "))
                .ok_or("tree 줄 없음")?
                .split_whitespace()
                .map(|x| x.parse().map_err(|_| "tree 값 오류".to_string()))
                .collect::<Result<_, _>>()?;
            let mut nodes = Vec::with_capacity(h[1]);
            for _ in 0..h[1] {
                let l = lines.next().ok_or("노드 부족")?;
                let p: Vec<&str> = l.split_whitespace().collect();
                let num = |i: usize| p.get(i).and_then(|x| x.parse::<f32>().ok()).ok_or("노드 값 오류".to_string());
                nodes.push(match p.first() {
                    Some(&"L") => Node { feat: -1, thr: 0.0, left: 0, right: 0, value: num(1)? },
                    Some(&"N") => Node {
                        feat: num(1)? as i32,
                        thr: num(2)?,
                        left: num(3)? as u32,
                        right: num(4)? as u32,
                        value: 0.0,
                    },
                    _ => return Err("노드 형식 오류".into()),
                });
            }
            trees.push(Tree { class: h[0], nodes });
        }
        Ok(Model { n_class, n_feat, trees, fast: Default::default() })
    }
}

// ───────────────────────────── 학습 ─────────────────────────────

#[derive(Clone, Debug)]
pub struct Params {
    pub rounds: usize,
    pub max_depth: usize,
    pub eta: f32,
    pub lambda: f32,
    pub gamma: f32,
    pub min_child_weight: f32,
    pub max_bins: usize,
}

impl Default for Params {
    fn default() -> Self {
        Params { rounds: 80, max_depth: 5, eta: 0.25, lambda: 1.0, gamma: 0.0, min_child_weight: 1.0, max_bins: 64 }
    }
}

/// 특징별 분할 경계 (cut[j]: x < cut[j] 이면 j 칸 이하)
struct Binned {
    cuts: Vec<Vec<f32>>,
    /// bins[f][i] = 표본 i 의 특징 f 칸 번호
    bins: Vec<Vec<u8>>,
}

fn bin_data(x: &[Vec<f32>], n_feat: usize, max_bins: usize) -> Binned {
    let mut cuts = Vec::with_capacity(n_feat);
    let mut bins = Vec::with_capacity(n_feat);
    for f in 0..n_feat {
        let mut vals: Vec<f32> = x.iter().map(|r| r[f]).collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
        vals.dedup();
        let reps: Vec<f32> = if vals.len() <= max_bins {
            vals
        } else {
            (0..max_bins).map(|i| vals[i * (vals.len() - 1) / (max_bins - 1)]).collect()
        };
        let c: Vec<f32> = reps.windows(2).map(|w| (w[0] + w[1]) / 2.0).collect();
        let b: Vec<u8> = x.iter().map(|r| c.partition_point(|&t| t <= r[f]) as u8).collect();
        cuts.push(c);
        bins.push(b);
    }
    Binned { cuts, bins }
}

struct Builder<'a> {
    data: &'a Binned,
    g: &'a [f32],
    h: &'a [f32],
    p: &'a Params,
    nodes: Vec<Node>,
}

impl Builder<'_> {
    fn leaf(&mut self, gs: f32, hs: f32) -> u32 {
        self.nodes.push(Node { feat: -1, thr: 0.0, left: 0, right: 0, value: -gs / (hs + self.p.lambda) * self.p.eta });
        (self.nodes.len() - 1) as u32
    }

    fn build(&mut self, idx: &mut [u32], depth: usize) -> u32 {
        let (gs, hs) = idx.iter().fold((0f32, 0f32), |(a, b), &i| (a + self.g[i as usize], b + self.h[i as usize]));
        if depth >= self.p.max_depth || idx.len() < 2 {
            return self.leaf(gs, hs);
        }
        let lam = self.p.lambda;
        let parent = gs * gs / (hs + lam);
        let mut best = (0f32, usize::MAX, 0usize); // (이득, 특징, 칸)
        for (f, cuts) in self.data.cuts.iter().enumerate() {
            if cuts.is_empty() {
                continue;
            }
            let nb = cuts.len() + 1;
            let mut hg = vec![0f32; nb];
            let mut hh = vec![0f32; nb];
            let col = &self.data.bins[f];
            for &i in idx.iter() {
                let b = col[i as usize] as usize;
                hg[b] += self.g[i as usize];
                hh[b] += self.h[i as usize];
            }
            let (mut gl, mut hl) = (0f32, 0f32);
            for j in 0..cuts.len() {
                gl += hg[j];
                hl += hh[j];
                let (gr, hr) = (gs - gl, hs - hl);
                if hl < self.p.min_child_weight || hr < self.p.min_child_weight {
                    continue;
                }
                let gain = 0.5 * (gl * gl / (hl + lam) + gr * gr / (hr + lam) - parent) - self.p.gamma;
                if gain > best.0 {
                    best = (gain, f, j);
                }
            }
        }
        if best.1 == usize::MAX || best.0 <= 1e-6 {
            return self.leaf(gs, hs);
        }
        let (f, j) = (best.1, best.2);
        let col = &self.data.bins[f];
        // 칸 번호 ≤ j → 왼쪽 (x < cuts[j])
        let mut k = 0;
        for t in 0..idx.len() {
            if (col[idx[t] as usize] as usize) <= j {
                idx.swap(t, k);
                k += 1;
            }
        }
        let me = self.nodes.len();
        self.nodes.push(Node { feat: f as i32, thr: self.data.cuts[f][j], left: 0, right: 0, value: 0.0 });
        let (l, r) = idx.split_at_mut(k);
        let li = self.build(l, depth + 1);
        let ri = self.build(r, depth + 1);
        self.nodes[me].left = li;
        self.nodes[me].right = ri;
        me as u32
    }
}

/// 학습. `y` 는 클래스 번호, `w` 는 표본 가중치(없으면 1).
pub fn fit(x: &[Vec<f32>], y: &[usize], n_class: usize, p: &Params, mut log: impl FnMut(usize, f32)) -> Model {
    let n = x.len();
    let n_feat = x[0].len();
    let data = bin_data(x, n_feat, p.max_bins);
    let mut margin = vec![vec![0f32; n_class]; n];
    let mut trees = Vec::new();
    let mut g = vec![0f32; n];
    let mut h = vec![0f32; n];
    for round in 0..p.rounds {
        let probs: Vec<Vec<f32>> = margin
            .iter()
            .map(|m| {
                let mut v = m.clone();
                softmax(&mut v);
                v
            })
            .collect();
        let loss = probs.iter().zip(y).map(|(pr, &c)| -pr[c].max(1e-9).ln()).sum::<f32>() / n as f32;
        log(round, loss);
        for k in 0..n_class {
            for i in 0..n {
                let pk = probs[i][k];
                g[i] = pk - if y[i] == k { 1.0 } else { 0.0 };
                h[i] = (2.0 * pk * (1.0 - pk)).max(1e-6);
            }
            let mut idx: Vec<u32> = (0..n as u32).collect();
            let mut b = Builder { data: &data, g: &g, h: &h, p, nodes: Vec::new() };
            b.build(&mut idx, 0);
            let tree = Tree { class: k, nodes: b.nodes };
            for i in 0..n {
                margin[i][k] += tree.score(&x[i]);
            }
            trees.push(tree);
        }
    }
    Model { n_class, n_feat, trees, fast: Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_interaction_and_roundtrips() {
        // 3클래스, 특징 상호작용: a 가 0 이면 0, a 가 1 이고 b 가 0 이면 1, 둘 다 1 이면 2 (c 는 잡음)
        let mut x = Vec::new();
        let mut y = Vec::new();
        for i in 0..600 {
            let (a, b) = ((i % 2) as f32, ((i / 2) % 2) as f32);
            x.push(vec![a, b, (i % 7) as f32]);
            y.push(if a == 0.0 { 0 } else if b == 0.0 { 1 } else { 2 });
        }
        let m = fit(&x, &y, 3, &Params { rounds: 20, max_depth: 3, ..Default::default() }, |_, _| {});
        assert!(m.predict_proba(&[0.0, 1.0, 3.0])[0] > 0.9);
        assert!(m.predict_proba(&[1.0, 0.0, 3.0])[1] > 0.9);
        assert!(m.predict_proba(&[1.0, 1.0, 5.0])[2] > 0.9);
        let p: f32 = m.predict_proba(&[1.0, 1.0, 5.0]).iter().sum();
        assert!((p - 1.0).abs() < 1e-5, "소프트맥스 합 = 1");
        let m2 = Model::from_text(&m.to_text()).unwrap();
        assert!((m2.predict_proba(&[1.0, 0.0, 1.0])[1] - m.predict_proba(&[1.0, 0.0, 1.0])[1]).abs() < 1e-4);
    }

    /// 분기 없는 평가가 노드 따라가기와 비트 단위로 같은 점수인가 — 내장 모델 2개 + 무작위 입력
    #[test]
    fn quickscorer_matches_node_walk() {
        let mut x = 0x1234_5678_9ABC_DEF1u64;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for m in [crate::rules::ml::model(), crate::rules::ml::natural_model()] {
            let q = QuickScorer::build(m).expect("잎 64개 이하");
            for _ in 0..20_000 {
                // 0/1 특징, 작은 정수, 실수, 임계값과 정확히 같은 값이 고루 섞이도록
                let v: Vec<f32> = (0..m.n_feat)
                    .map(|f| match rnd() % 4 {
                        0 => (rnd() % 2) as f32,
                        1 => (rnd() % 20) as f32,
                        2 => (rnd() % 10_000) as f32 / 1000.0,
                        _ => m.trees[(rnd() % m.trees.len() as u64) as usize].nodes.iter().find(|n| n.feat == f as i32).map_or(0.0, |n| n.thr),
                    })
                    .collect();
                let (a, b) = (q.margins(&v, m.n_class), m.margins_by_nodes(&v));
                assert_eq!(a.iter().map(|f| f.to_bits()).collect::<Vec<_>>(), b.iter().map(|f| f.to_bits()).collect::<Vec<_>>());
            }
        }
    }
}

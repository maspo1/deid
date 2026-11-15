//! 파일 분석 트리 모델 (순수 데이터) — egui 판 `gui/forensic.rs` 와 같은 규칙.
//! 폴더 → 파일 → [펼친 파일만] 정보 타입 묶음 → 값. 화면은 이 평탄화된 행 목록을 가상 리스트뷰로 보여준다.
//! (egui 판은 macOS 용으로 그대로 두므로 모델을 공유하지 않고 같은 규칙으로 따로 둔다)

use crate::gui::scan::{Scan, Src};
use crate::ipc::{Rec, Status};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Default, Clone, Copy, Debug)]
pub struct Agg {
    pub files: usize,
    pub recs: usize,
    pub review: usize,
    pub masked: usize,
    pub size: u64,
}

impl Agg {
    fn add(&mut self, o: &Agg) {
        self.files += o.files;
        self.recs += o.recs;
        self.review += o.review;
        self.masked += o.masked;
        self.size += o.size;
    }
}

#[derive(Clone)]
pub enum Kind {
    /// 상대 경로 (접기 키)
    Dir(String),
    File(usize),
    /// 정보 타입 묶음 (접기 키, 파일 번호)
    Type(String, usize),
    /// 값 1건 (파일 번호, 위치, 번호)
    Item(usize, Src, usize),
}

#[derive(Clone)]
pub struct Node {
    pub name: String,
    pub depth: usize,
    pub kind: Kind,
    pub agg: Agg,
    /// 이름에서 나온 검출이 있음
    pub name_hit: Option<usize>,
}

impl Node {
    /// 펼치기 상자가 있는 행인가
    pub fn expandable(&self) -> bool {
        match self.kind {
            Kind::Dir(_) | Kind::Type(..) => true,
            Kind::File(_) => self.agg.recs > 0,
            Kind::Item(..) => false,
        }
    }
}

pub fn type_key(file: usize, label: &str) -> String {
    format!("\x01{file}:{label}")
}

fn review_count(r: &[Rec]) -> usize {
    r.iter().filter(|x| x.review).count()
}

#[derive(Default)]
struct DirTree {
    dirs: BTreeMap<String, DirTree>,
    files: Vec<(String, usize)>,
}

/// 전체 행 목록 (접힌 것 포함). `query` 는 소문자.
pub fn build(scan: &Scan, only_found: bool, query: &str, expanded: &HashSet<usize>) -> Vec<Node> {
    let mut root = DirTree::default();
    for (i, e) in scan.entries.iter().enumerate() {
        if only_found && e.records.is_empty() {
            continue;
        }
        let rel = scan.rel(&e.path);
        if !query.is_empty() && !rel.to_lowercase().contains(query) {
            continue;
        }
        let parts: Vec<&str> = rel.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
        let Some((file, dirs)) = parts.split_last() else { continue };
        let mut node = &mut root;
        for d in dirs {
            node = node.dirs.entry(d.to_string()).or_default();
        }
        node.files.push((file.to_string(), i));
    }
    let names: HashMap<String, usize> = scan.names.iter().enumerate().map(|(i, n)| (scan.rel(&n.path), i)).collect();
    let mut out = Vec::new();
    flatten(&root, "", 0, scan, &names, expanded, &mut out);
    out
}

fn flatten(t: &DirTree, prefix: &str, depth: usize, scan: &Scan, names: &HashMap<String, usize>, expanded: &HashSet<usize>, out: &mut Vec<Node>) -> Agg {
    let mut total = Agg::default();
    for (name, sub) in &t.dirs {
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}{}{name}", std::path::MAIN_SEPARATOR) };
        let at = out.len();
        let name_hit = names.get(&rel).copied();
        out.push(Node { name: name.clone(), depth, kind: Kind::Dir(rel.clone()), agg: Agg::default(), name_hit });
        let mut agg = flatten(sub, &rel, depth + 1, scan, names, expanded, out);
        if let Some(n) = name_hit {
            agg.recs += scan.names[n].records.len();
            agg.review += review_count(&scan.names[n].records);
        }
        out[at].agg = agg;
        total.add(&agg);
    }
    for (name, i) in &t.files {
        let e = &scan.entries[*i];
        let name_hit = names.get(&scan.rel(&e.path)).copied();
        let mut agg = Agg { files: 1, recs: e.records.len(), review: review_count(&e.records), masked: (e.status == Status::Masked) as usize, size: e.size };
        if let Some(n) = name_hit {
            agg.recs += scan.names[n].records.len();
            agg.review += review_count(&scan.names[n].records);
        }
        out.push(Node { name: name.clone(), depth, kind: Kind::File(*i), agg, name_hit });
        if expanded.contains(i) {
            push_types(*i, name_hit, depth + 1, scan, out);
        }
        total.add(&agg);
    }
    total
}

fn push_types(file: usize, name_hit: Option<usize>, depth: usize, scan: &Scan, out: &mut Vec<Node>) {
    let mut srcs = vec![Src::File(file)];
    srcs.extend(name_hit.map(Src::Name));
    let mut by: BTreeMap<&str, Vec<(Src, usize)>> = BTreeMap::new();
    for s in srcs {
        for (k, r) in scan.records(s).iter().enumerate() {
            by.entry(r.base()).or_default().push((s, k));
        }
    }
    for (label, items) in by {
        let review = items.iter().filter(|(s, k)| scan.records(*s)[*k].review).count();
        out.push(Node {
            name: format!("{label} ({})", items.len()),
            depth,
            kind: Kind::Type(type_key(file, label), file),
            agg: Agg { recs: items.len(), review, ..Default::default() },
            name_hit: None,
        });
        for (s, k) in items {
            let r = &scan.records(s)[k];
            out.push(Node {
                name: format!("{} → {}", r.original, r.masked),
                depth: depth + 1,
                kind: Kind::Item(file, s, k),
                agg: Agg { recs: 1, review: r.review as usize, ..Default::default() },
                name_hit: None,
            });
        }
    }
}

/// 접힌 폴더·묶음의 하위를 뺀 보이는 행 번호
pub fn visible(nodes: &[Node], collapsed: &HashSet<String>) -> Vec<u32> {
    let mut out = Vec::with_capacity(nodes.len());
    let mut hide_below: Option<usize> = None;
    for (i, n) in nodes.iter().enumerate() {
        if let Some(d) = hide_below {
            if n.depth > d {
                continue;
            }
            hide_below = None;
        }
        out.push(i as u32);
        if let Kind::Dir(p) | Kind::Type(p, _) = &n.kind {
            if collapsed.contains(p) {
                hide_below = Some(n.depth);
            }
        }
    }
    out
}

pub fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ipc::FileEntry;

    /// 테스트용 검사 결과: d0/·d1/ 에 파일 n개, 3개마다 검출 2건(1건 검토필요)
    pub fn sample(n: usize) -> Scan {
        let root = std::path::PathBuf::from("C:/in");
        let mut s = Scan::new(root.clone());
        for i in 0..n {
            let records = if i % 3 == 0 {
                vec![
                    Rec { original: format!("010-1234-{i:04}"), masked: "***-****-****".into(), label: "핸드폰번호".into(), review: false },
                    Rec { original: "10.0.0.1".into(), masked: "**.*.*.*".into(), label: "IP주소(검토필요)".into(), review: true },
                ]
            } else {
                vec![]
            };
            s.entries.push(FileEntry {
                path: root.join(format!("d{}/f{i:03}.txt", i % 2)),
                out: root.join("o.txt"),
                kind: "TEXT(UTF-8)".into(),
                size: 100,
                status: if records.is_empty() { Status::Clean } else { Status::Masked },
                notes: vec![],
                records,
            });
        }
        s.version = 1;
        s
    }

    #[test]
    fn tree_groups_by_type_when_expanded() {
        let s = sample(6);
        let nodes = build(&s, false, "", &HashSet::new());
        let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["d0", "f000.txt", "f002.txt", "f004.txt", "d1", "f001.txt", "f003.txt", "f005.txt"]);
        assert_eq!(nodes[0].agg.recs, 2);
        let nodes = build(&s, false, "", &HashSet::from([0]));
        let names: Vec<&str> = nodes.iter().take(6).map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["d0", "f000.txt", "IP주소 (1)", "10.0.0.1 → **.*.*.*", "핸드폰번호 (1)", "010-1234-0000 → ***-****-****"]);
        // 묶음 접기 → 값 숨김
        let vis = visible(&nodes, &HashSet::from([type_key(0, "IP주소")]));
        assert!(!vis.iter().any(|&i| nodes[i as usize].name.starts_with("10.0.0.1")));
        // 폴더 접기 → 하위 전부 숨김
        let vis = visible(&nodes, &HashSet::from(["d0".to_string()]));
        assert_eq!(nodes[vis[1] as usize].name, "d1");
    }

    #[test]
    fn filters() {
        let s = sample(9);
        assert_eq!(build(&s, true, "", &HashSet::new()).iter().filter(|n| matches!(n.kind, Kind::File(_))).count(), 3);
        assert_eq!(build(&s, false, "f004", &HashSet::new()).len(), 2); // d0 + f004
    }
}

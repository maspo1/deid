//! ML 모델 학습 (순수 Rust): `cargo run --release --example train_ml`
//!
//! 1단 자연스러움 모델 (실제 IP 분포 vs 사람이 만든 배열) → src/rules/ml/natural.txt
//! 2단 본 모델 (변형 번호 + 맥락 + 문서 단위 흔적) → src/rules/ml/model.txt
//!    학습 → 새 정답·오답 표본을 주입해 틀린 것만 골라 다시 넣고 재학습 (어려운 표본 주입, 모든 클래스)
//! 각 단계는 학습과 다른 seed 의 데이터로 평가하고, 사람 직관 점검 문장의 판단을 출력한다.

use deid::rules::ml::features::{CLASSES, N_FEATURES};
use deid::rules::ml::intuition::{natural_dataset, N_NATURAL};
use deid::rules::ml::{features, gbdt, synth};

fn confusion(model: &gbdt::Model, x: &[Vec<f32>], y: &[usize], names: &[&str]) {
    let k = names.len();
    let mut cm = vec![vec![0usize; k]; k];
    let mut logloss = 0.0;
    for (xi, &yi) in x.iter().zip(y) {
        let p = model.predict_proba(xi);
        logloss -= p[yi].max(1e-9).ln();
        let pred = p.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
        cm[yi][pred] += 1;
    }
    println!("  평가 logloss {:.4}  (표본 {})", logloss / x.len() as f32, x.len());
    println!("  혼동 행렬 (행=정답, 열=예측): {names:?}");
    for (i, row) in cm.iter().enumerate() {
        println!("    {:<6} {:?}", names[i], row);
    }
    for c in 0..k {
        let tp = cm[c][c] as f32;
        let pred: usize = cm.iter().map(|r| r[c]).sum();
        let real: usize = cm[c].iter().sum();
        println!("    {:<6} 정밀도 {:.3}  재현율 {:.3}", names[c], tp / pred.max(1) as f32, tp / real.max(1) as f32);
    }
}

fn main() {
    let t0 = std::time::Instant::now();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/rules/ml");

    // ── 1단: 자연스러움 ──
    println!("[1단] 자연스러움 모델 — 실제 IP(사설·공인 사전) vs 사람이 만든 배열, 특징 {N_NATURAL}개");
    let (x1, y1) = natural_dataset(11, 40_000);
    let (tx1, ty1) = natural_dataset(12, 10_000);
    let p1 = gbdt::Params { rounds: 60, max_depth: 5, ..Default::default() };
    let nat = gbdt::fit(&x1, &y1, 2, &p1, |_, _| {});
    confusion(&nat, &tx1, &ty1, &["인공", "실제"]);
    std::fs::write(dir.join("natural.txt"), nat.to_text()).unwrap();

    println!("\n  사람 직관 점검 (실제 IP 같을 확률):");
    for o in [
        [12, 34, 56, 78],
        [123, 45, 6, 78],
        [10, 20, 30, 40],
        [1, 2, 3, 4],
        [11, 22, 33, 44],
        [100, 200, 50, 0],
        [192, 168, 0, 1],
        [192, 168, 10, 23],
        [10, 0, 3, 15],
        [168, 126, 63, 1],
        [211, 234, 10, 5],
        [57, 203, 14, 98],
        [8, 8, 8, 8],
    ] {
        let p = nat.predict_proba(&deid::rules::ml::intuition::natural_features(&o))[1];
        println!("    {:<16} {p:.3}", format!("{}-{}-{}-{}", o[0], o[1], o[2], o[3]));
    }

    // ── 2단: 본 모델 ──
    println!("\n[2단] 본 모델 — 특징 {N_FEATURES}개 (후보 + 자연스러움 + 사전 + 문서 단위 기계적 치환 흔적 + 다의성)");
    let (mut x, mut y) = synth::dataset(1, 40_000, &nat);
    let (sx, sy) = synth::snippet_dataset(3, 40_000, &nat);
    x.extend(sx);
    y.extend(sy);
    let (tx, ty) = synth::dataset(2, 10_000, &nat);
    let mut counts = [0usize; 5];
    y.iter().for_each(|&c| counts[c] += 1);
    println!("  학습 표본 {} (클래스별 {:?})", x.len(), counts);
    let p2 = gbdt::Params { rounds: 120, max_depth: 6, eta: 0.2, ..Default::default() };
    let fit = |x: &[Vec<f32>], y: &[usize]| {
        gbdt::fit(x, y, CLASSES.len(), &p2, |r, loss| {
            if r % 40 == 0 {
                println!("    round {r:>3}  logloss {loss:.4}");
            }
        })
    };
    let mut model = fit(&x, &y);
    let argmax = |p: &[f32]| p.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
    // 어려운 표본 주입: 매번 새 seed 의 정답·오답을 만들어 지금 모델이 틀린 것만 학습에 더한다 (2배 가중)
    for k in 0..3u64 {
        let (mut px, mut py) = synth::dataset(100 + k, 15_000, &nat);
        let (sx, sy) = synth::snippet_dataset(200 + k, 40_000, &nat);
        px.extend(sx);
        py.extend(sy);
        let wrong: Vec<usize> = (0..px.len()).filter(|&i| argmax(&model.predict_proba(&px[i])) != py[i]).collect();
        let mut per = [0usize; 5];
        wrong.iter().for_each(|&i| per[py[i]] += 1);
        println!("  주입 {}: 새 표본 {} 중 틀린 것 {} ({:.2}%, 정답 클래스별 {per:?}) → 재학습", k + 1, px.len(), wrong.len(), 100.0 * wrong.len() as f32 / px.len() as f32);
        for &i in &wrong {
            for _ in 0..2 {
                x.push(px[i].clone());
                y.push(py[i]);
            }
        }
        model = fit(&x, &y);
    }
    confusion(&model, &tx, &ty, &CLASSES);
    std::fs::write(dir.join("model.txt"), model.to_text()).unwrap();

    println!("\n  사람 직관 점검 (후보별 최고 클래스와 확률):");
    for t in [
        "0105678-1234",
        "12-34-56-78",
        "123-45-6-78",
        "10-20-30-40",
        "11-22-33-44",
        "ip=12-34-56-78",
        "ip=123-45-6-78",
        "192-168-0-1",
        "168-126-63-1",
        "server 211-234-10-5",
        "57-203-14-98",
        "item 57-203-14-98",
        "(일정) 10.26.~10.31.",
        "login from 211-234-10-5 user kim\nlogin from 58-120-33-7 user lee\nlogin from 192-168-0-15 user park",
        "code 12-34-56-78\ncode 23-45-67-89\ncode 34-56-78-90",
        "hp = 0109999-1234, server 192-168-0-1, code 12-34-56-78",
        "login from 211-234-10-5\nlogin from 12-34-56-78\nlogin from 58-120-33-7",
        // 실제 문서 오답 (정답 없음)
        "참고 28-35, 97-98",
        "205-1~205-3",
        "152.1 145 150",
        "00013027665 14",
        "511.05667 -625.17676",
        "44610,1903,2491,855278",
        "122.90 -> 3.1",
        "IMG_20240117_120712.png",
        // 기호가 바뀌거나 빠진 정보 (맥락으로 맞춰야 함)
        "서버 IP 122->90->3->1",
        "연락처 010→1234→5678",
        "주민번호 8811111234569",
        // 레코드 묶음 (복원 가능) vs 하나의 규칙으로 떼어낼 수 없는 덩어리
        "88.11.25.1,14.20.1.3",
        "88^11^25*1,88&11$25*1",
        "[88^11^25*1][88&11$25*1]",
        "88-11-25-1;14-20-1-3;201-33-7-90",
        "21.0.88.11.25.1.7",
        "3^4*55-12^9*200-1",
        // 숫자표 열 구분자로 같은 규칙 (맥락 없음) vs 표 한 줄
        "215|1|225|123",
        "no 203 105 180 171",
        "<td>10 | 43 | 198 | 163",
        "21,23,53,60",
    ] {
        let nat_ref = &nat;
        let mut parts = Vec::new();
        for (c, v) in features::doc_features(t, nat_ref) {
            if features::quad(&c).is_none() && c.groups.len() < 2 {
                continue;
            }
            let p = model.predict_proba(&v);
            let (k, pk) = p.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
            parts.push(format!("{}={}:{:.2}", &t[c.start..c.end], CLASSES[k], pk));
        }
        println!("    {:<40} {}", t.replace('\n', " ⏎ ").chars().take(40).collect::<String>(), parts.join("  "));
    }
    println!("\n저장: natural.txt (트리 {}), model.txt (트리 {}) — {:.1}초", nat.trees.len(), model.trees.len(), t0.elapsed().as_secs_f32());
}

//! ML 모델 학습 (순수 Rust): `cargo run --release --example train_ml`
//!
//! 1단 자연스러움 모델 (실제 IP 분포 vs 사람이 만든 배열) → src/rules/ml/natural.txt
//! 2단 본 모델 (변형 번호 + 맥락 + 문서 단위 흔적) → src/rules/ml/model.txt
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
    let (x, y) = synth::dataset(1, 40_000, &nat);
    let (tx, ty) = synth::dataset(2, 10_000, &nat);
    let mut counts = [0usize; 5];
    y.iter().for_each(|&c| counts[c] += 1);
    println!("  학습 표본 {} (클래스별 {:?})", x.len(), counts);
    let p2 = gbdt::Params { rounds: 120, max_depth: 6, eta: 0.2, ..Default::default() };
    let model = gbdt::fit(&x, &y, CLASSES.len(), &p2, |r, loss| {
        if r % 20 == 0 {
            println!("    round {r:>3}  logloss {loss:.4}");
        }
    });
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

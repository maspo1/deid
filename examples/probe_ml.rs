//! ML 판단 진단: `cargo run --release --example probe_ml -- "server 211-234-10-5" "..."`
//! 후보별 클래스 확률과 주요 특징을 출력한다.

use deid::rules::ml::features::{self, CLASSES, N_BASE};
use deid::rules::ml::{model, natural_model};

fn main() {
    for t in std::env::args().skip(1) {
        let t = t.replace("\\n", "\n");
        println!("== {t:?}");
        for (c, v) in features::doc_features(&t, natural_model()) {
            let p = model().predict_proba(&v);
            let probs: Vec<String> = CLASSES.iter().zip(&p).map(|(n, x)| format!("{n}={x:.2}")).collect();
            println!(
                "  {:<22} {}  | p_nat={:.2} ctx_ip={} same_sig_ip={} ambiguity={} restorable={}",
                &t[c.start..c.end],
                probs.join(" "),
                v[N_BASE + 1],
                v[37],
                v[N_BASE + 11],
                v[N_BASE + 15],
                v[N_BASE + 10],
            );
        }
    }
}

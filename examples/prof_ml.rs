//! ML 단계별 시간: `cargo run --release --example prof_ml -- <파일>`
use deid::rules::ml::{self, features};
use std::time::Instant;
fn main() {
    let text = String::from_utf8_lossy(&std::fs::read(std::env::args().nth(1).unwrap()).unwrap()).into_owned();
    let t = Instant::now();
    let c = features::candidates(&text);
    println!("candidates {} 개 {:.0} ms", c.len(), t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    for x in &c {
        std::hint::black_box(features::features(&text, x));
    }
    println!("features() {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let f = features::doc_features(&text, ml::natural_model());
    println!("doc_features {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    for (_, v) in &f {
        std::hint::black_box(ml::model().predict_proba(v));
    }
    println!("predict {:.0} ms ({} trees)", t.elapsed().as_secs_f64() * 1e3, ml::model().trees.len());
}

//! 텍스트 검사 단계별 시간: `cargo run --release --example prof_text -- <파일> [반복]`

use deid::config::Options;
use deid::rules::{context::CredentialScanner, ml, patterns::Patterns, Detector, Lang};
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("파일");
    let rep: usize = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(5);
    let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
    let mb = text.len() as f64 * rep as f64 / 1e6;
    let o = Options::default();
    let p = Patterns::new();
    let cr = CredentialScanner::new();
    let det = Detector::new(o.clone());
    let time = |name: &str, f: &dyn Fn(&mut Vec<deid::common::Hit>)| {
        let mut v = Vec::new();
        f(&mut v);
        let t = Instant::now();
        for _ in 0..rep {
            v.clear();
            f(&mut v);
        }
        let s = t.elapsed().as_secs_f64();
        println!("{name:<18} {:>8.1} ms  {:>7.1} MB/s  hits {}", s * 1000.0 / rep as f64, mb / s, v.len());
    };
    time("phones", &|v| p.phones(&text, v));
    time("phones_mech", &|v| p.phones_mechanical(&text, v));
    time("cards", &|v| p.cards(&text, v));
    time("emails", &|v| p.emails(&text, v));
    time("rrns", &|v| p.rrns(&text, v));
    time("ipv4s", &|v| p.ipv4s(&text, v));
    time("macs", &|v| p.macs(&text, v));
    time("ml::scan", &|v| ml::scan(&text, &o, v));
    time("creds", &|v| cr.scan(&text, o.threshold, v));
    time("candidates", &|v| { let n = ml::features::candidates(&text).len(); if n == usize::MAX { v.clear() } });
    if let Some(img) = std::env::args().nth(3) {
        let b = std::fs::read(img).unwrap();
        let t = Instant::now();
        let out = deid::handlers::raster::blur_whole_bytes(&b, 1.0).unwrap();
        println!("blur_whole_bytes {:.0} ms ({} B)", t.elapsed().as_secs_f64() * 1000.0, out.len());
    }
    let rr = regex::Regex::new(r"[0-9]+(?:[^\p{L}\p{N}\r\n]{1,3}[0-9]+)*").unwrap();
    time("run_regex only", &|v| { let n = rr.find_iter(&text).count(); if n == usize::MAX { v.clear() } });
    let mx = deid::rules::matrix::Matrix::new();
    time("matrix (5종+IP)", &|v| mx.scan(&text, |_| true, v));
    time("matrix (IP 끔)", &|v| mx.scan(&text, |k| *k != deid::common::InfoType::Ip, v));
    time("matrix (전부 끔)", &|v| mx.scan(&text, |_| false, v));
    let mut all = Vec::new();
    mx.scan(&text, |_| true, &mut all);
    ml::scan(&text, &o, &mut all);
    cr.scan(&text, o.threshold, &mut all);
    p.phones_mechanical(&text, &mut all);
    let t = Instant::now();
    let n = deid::common::resolve_overlaps(all.clone()).len();
    println!("resolve_overlaps   {:>8.1} ms  ({} → {n})", t.elapsed().as_secs_f64() * 1e3, all.len());
    time("Detector::scan", &|v| v.extend(det.scan(&text, Lang::None)));
}

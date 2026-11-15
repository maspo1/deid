//! OCR 진단: 그림 하나를 배율별로 인식한 원문 줄과, 현재 규칙으로 찾은 정보를 출력한다.
//!
//! `cargo run --release --example probe_ocr -- <그림> [배율,배율..]`

use deid::config::Options;
use deid::handlers::raster;
use deid::ocr::recognize_scaled;
use deid::rules::Detector;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().expect("그림 경로");
    let scales: Vec<f32> = args.get(1).map_or(vec![1.0, 2.0, 3.0], |s| s.split(',').filter_map(|x| x.parse().ok()).collect());
    let img = image::open(path).expect("그림 열기").to_rgba8();
    println!("{path}: {}x{}", img.width(), img.height());
    for &s in &scales {
        println!("\n===== 배율 {s} =====");
        for l in recognize_scaled(&img, s).expect("OCR") {
            let t: Vec<&str> = l.words.iter().map(|w| w.text.as_str()).collect();
            let y = l.words.first().map_or(0.0, |w| w.y);
            let x = l.words.first().map_or(0.0, |w| w.x);
            println!("({x:>6.0},{y:>5.0}) {}", t.join(" "));
        }
        let mut o = Options::default();
        o.depth = std::env::var("DEPTH").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        o.ocr_scales = std::env::var("SCALES").map_or(vec![s], |v| v.split(',').filter_map(|x| x.parse().ok()).collect());
        let hits = raster::detect(&img, &Detector::new(o)).expect("검출");
        println!("-- 검출 {}건", hits.len());
        for h in &hits {
            println!("   ({:>6.0},{:>5.0}) {:<20} {}", h.rect.x, h.rect.y, h.record.original, h.record.label());
        }
    }
}

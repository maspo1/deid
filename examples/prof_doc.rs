//! 단계별 시간 측정: 문서(zip) 속 그림마다 디코딩 / 전처리 / OCR 호출 / 텍스트 검사 / 흐림 / 인코딩.
//!
//! `cargo run --release --example prof_doc -- <문서.hwpx|docx|그림>`

use deid::config::Options;
use deid::handlers::raster;
use deid::ocr::{effective_scale, recognize_scaled};
use deid::rules::{Detector, Lang};
use std::io::Read;
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let path = std::env::args().nth(1).expect("경로");
    let bytes = std::fs::read(&path).unwrap();
    let mut imgs: Vec<(String, Vec<u8>)> = Vec::new();
    let mut texts: Vec<(String, String)> = Vec::new();
    if let Ok(mut ar) = zip::ZipArchive::new(std::io::Cursor::new(&bytes)) {
        for i in 0..ar.len() {
            let mut f = ar.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut b = Vec::new();
            f.read_to_end(&mut b).unwrap();
            if image::guess_format(&b).is_ok() {
                imgs.push((name, b));
            } else if let Ok(s) = String::from_utf8(b) {
                texts.push((name, s));
            }
        }
    } else {
        imgs.push((path.clone(), bytes));
    }
    let det = Detector::new(Options::default());
    for (name, s) in &texts {
        let t = Instant::now();
        let n = det.scan(s, Lang::None).len();
        println!("TEXT {name:<28} {:>9} B  scan {:>8.1} ms  hits {n}", s.len(), ms(t));
    }
    let o = &det.opts;
    let (mut t_dec, mut t_pre, mut t_ocr, mut t_scan, mut t_blur, mut t_enc) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for (name, b) in &imgs {
        let fmt = image::guess_format(b).unwrap();
        let t = Instant::now();
        let mut img = image::load_from_memory_with_format(b, fmt).unwrap().to_rgba8();
        let dec = ms(t);
        let t = Instant::now();
        let enh = deid::ocr::enhance(&img);
        let pre = ms(t);
        let mut ocr = String::new();
        let mut ocr_sum = 0.0;
        let mut scan_sum = 0.0;
        let mut done: Vec<f32> = vec![];
        for &req in &o.ocr_scales {
            let s = effective_scale(&enh, req, o.ocr_max_pixels).unwrap();
            if done.iter().any(|d| (d - s).abs() < 0.1) {
                continue;
            }
            done.push(s);
            let t = Instant::now();
            let lines = recognize_scaled(&enh, s).unwrap();
            let d = ms(t);
            ocr_sum += d;
            ocr += &format!(" x{s:.2}:{d:.0}ms/{}줄", lines.len());
            let t = Instant::now();
            for l in &lines {
                let txt: Vec<&str> = l.words.iter().map(|w| w.text.as_str()).collect();
                det.scan(&txt.join(" "), Lang::None);
            }
            scan_sum += ms(t);
        }
        let t = Instant::now();
        let hits = raster::detect(&img, &det).unwrap();
        let full_detect = ms(t);
        let t = Instant::now();
        for h in &hits {
            raster::blur_rect(&mut img, h.rect, o.blur_strength);
        }
        let blur = ms(t);
        let t = Instant::now();
        let enc_len = if hits.is_empty() { 0 } else { raster::encode(&img, fmt, false).unwrap().len() };
        let enc = if hits.is_empty() { 0.0 } else { ms(t) };
        println!(
            "IMG {name:<22} {:>5}x{:<5} dec {dec:>7.0} pre {pre:>6.0} ocr{ocr}  scan {scan_sum:.1}  detect(전체) {full_detect:>6.0}  hits {:>2} blur {blur:>6.0} enc {enc:>6.0} ({enc_len}B)",
            img.width(),
            img.height(),
            hits.len()
        );
        t_dec += dec;
        t_pre += pre;
        t_ocr += ocr_sum;
        t_scan += scan_sum;
        t_blur += blur;
        t_enc += enc;
    }
    println!("합계 ms: dec {t_dec:.0} pre {t_pre:.0} ocr {t_ocr:.0} scan {t_scan:.1} blur {t_blur:.0} enc {t_enc:.0}");
}

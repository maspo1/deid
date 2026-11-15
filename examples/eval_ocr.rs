//! 그림 OCR 검출 평가 — 정답(위치 열 + IP) 대비 덮음률(흐림이 그 값을 덮었는가)·값 정확률을 설정 조합별로 잰다.
//!
//! `cargo run --release --example eval_ocr -- <그림> <정답파일> [배율목록;배율목록..]`
//! 정답 파일: 줄마다 `x최소 x최대 IP` (그 IP 가 찍힌 열의 가로 범위, 원본 화소)

use deid::config::Options;
use deid::handlers::raster;
use deid::rules::Detector;
use image::RgbaImage;

fn digits(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// (덮음, 값 정확, 전체, 검출 건수). `imgs` 여러 개면 결과를 합친다 (같은 자리 중복은 1건)
fn score(imgs: &[RgbaImage], truth: &[(f32, f32, String)], scales: &[f32]) -> (usize, usize, usize, usize) {
    let mut o = Options::default();
    o.ocr_scales = scales.to_vec();
    o.ocr_max_pixels = u64::MAX / 4;
    o.ocr_enhance = std::env::var("ENH").map_or(true, |v| v != "0");
    o.depth = std::env::var("DEPTH").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    // LITE=0 → 1차 검사도 Windows OCR (이전 방식)
    o.ocr_lite = std::env::var("LITE").map_or(true, |v| v != "0");
    let det = Detector::new(o);
    let mut hits: Vec<raster::ImageHit> = Vec::new();
    for im in imgs {
        for h in raster::detect(im, &det).expect("검출") {
            let r = h.rect;
            let dup = hits.iter().any(|f| {
                let w = (f.rect.x + f.rect.w).min(r.x + r.w) - f.rect.x.max(r.x);
                let hh = (f.rect.y + f.rect.h).min(r.y + r.h) - f.rect.y.max(r.y);
                w > 0.0 && hh > 0.0 && w * hh > 0.3 * (r.w * r.h).min(f.rect.w * f.rect.h)
            });
            if !dup {
                hits.push(h);
            }
        }
    }
    let mut used = vec![false; hits.len()];
    let (mut cover, mut exact) = (0, 0);
    for (x0, x1, ip) in truth {
        let d = digits(ip);
        let pick = hits.iter().enumerate().position(|(i, h)| {
            let cx = h.rect.x + h.rect.w / 2.0;
            let hd = digits(&h.record.original);
            !used[i] && cx >= *x0 && cx <= *x1 && (hd == d || hd == format!("1{d}"))
        });
        if pick.is_none() && std::env::var("MISS").is_ok() {
            println!("   놓침: x{x0}~{x1} {ip}");
        }
        if let Some(i) = pick {
            used[i] = true;
            cover += 1;
            exact += (hits[i].record.original == *ip) as usize;
        }
    }
    (cover, exact, truth.len(), hits.len())
}

/// 실험용 전처리
fn pre(img: &RgbaImage, kind: &str) -> RgbaImage {
    let lum = |p: &image::Rgba<u8>| (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
    let mut o = img.clone();
    match kind {
        "gray" => {
            for p in o.pixels_mut() {
                let l = lum(p) as u8;
                *p = image::Rgba([l, l, l, 255]);
            }
        }
        "bin" => {
            // 오츠 이진화
            let mut hist = [0u64; 256];
            for p in img.pixels() {
                hist[lum(p) as usize] += 1;
            }
            let total: u64 = hist.iter().sum();
            let sum: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
            let (mut wb, mut sb, mut best, mut th) = (0f64, 0f64, 0f64, 128u32);
            for t in 0..256 {
                wb += hist[t] as f64;
                if wb == 0.0 || wb == total as f64 {
                    continue;
                }
                sb += t as f64 * hist[t] as f64;
                let wf = total as f64 - wb;
                let (mb, mf) = (sb / wb, (sum - sb) / wf);
                let v = wb * wf * (mb - mf).powi(2);
                if v > best {
                    best = v;
                    th = t as u32;
                }
            }
            for p in o.pixels_mut() {
                let v = if lum(p) > th { 255 } else { 0 };
                *p = image::Rgba([v, v, v, 255]);
            }
        }
        "sharp" => o = image::imageops::unsharpen(img, 1.0, 5),
        "unbar" => {
            // 글자보다 굵고 긴 세로 막대(그래프·표 선) 지우기: 어두운 세로 연속이 14px 이상이고 가로 두께 3px 이상
            let (w, h) = img.dimensions();
            let dark = |x: u32, y: u32| lum(img.get_pixel(x, y)) < 160;
            let mut run = vec![0u32; (w * h) as usize];
            for x in 0..w {
                let mut y = 0;
                while y < h {
                    if !dark(x, y) {
                        y += 1;
                        continue;
                    }
                    let s = y;
                    while y < h && dark(x, y) {
                        y += 1;
                    }
                    for yy in s..y {
                        run[(yy * w + x) as usize] = y - s;
                    }
                }
            }
            for y in 0..h {
                for x in 0..w {
                    let tall = |xx: u32| run[(y * w + xx) as usize] >= 14;
                    if tall(x) && ((x >= 2 && tall(x - 1) && tall(x - 2)) || (x >= 1 && x + 1 < w && tall(x - 1) && tall(x + 1)) || (x + 2 < w && tall(x + 1) && tall(x + 2))) {
                        o.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
                    }
                }
            }
        }
        _ => {}
    }
    o
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let img = image::open(&a[0]).expect("그림").to_rgba8();
    let truth: Vec<(f32, f32, String)> = std::fs::read_to_string(&a[1])
        .expect("정답")
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            (p.len() == 3).then(|| (p[0].parse().unwrap(), p[1].parse().unwrap(), p[2].to_string()))
        })
        .collect();
    let sets = a.get(2).map_or("3,2;2;3;4;2.5;1.5;3,2,1.5;4,2", |s| s.as_str());
    let pres = a.get(3).map_or("none", |s| s.as_str());
    println!("{}: {}x{}, 정답 {}곳", a[0], img.width(), img.height(), truth.len());
    println!("{:<18} {:<12} {:>8} {:>8} {:>6}", "전처리", "배율", "덮음률", "값정확", "검출");
    for p in pres.split(',') {
        let ims: Vec<RgbaImage> = p.split('|').map(|v| v.split('+').fold(img.clone(), |acc, k| pre(&acc, k))).collect();
        if let Ok(d) = std::env::var("DUMP") {
            ims[0].save(&d).expect("저장");
        }
        for set in sets.split(';') {
            let scales: Vec<f32> = set.split(',').filter_map(|x| x.parse().ok()).collect();
            let t = std::time::Instant::now();
            let (c, e, n, h) = score(&ims, &truth, &scales);
            println!("{:<18} {:<12} {:>7.3} {:>8.3} {:>6}  ({:.1}초)", p, set, c as f32 / n as f32, e as f32 / n as f32, h, t.elapsed().as_secs_f32());
        }
    }
}

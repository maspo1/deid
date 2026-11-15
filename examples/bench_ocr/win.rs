//! OCR 엔진 비교 벤치마크 — 정답을 아는 문서형 그림을 합성해 두 엔진의 검출률·오탐·시간을 잰다.
//!
//! `cargo run --release --example bench_ocr -- gen <폴더> [장수]`   그림 + 정답 파일 생성 (GDI, 학습과 다른 씨앗·문장 배치)
//! `cargo run --release --example bench_ocr -- run <폴더> lite|win`  검출 → 덮음률(정답 상자를 흐림이 덮음)·값 정확·오탐
//!
//! 정답 파일(`*.txt`): 줄마다 `x0 y0 x1 y1 종류 값`

use deid::config::Options;
use deid::handlers::raster;
use deid::rules::Detector;
use image::RgbaImage;
use std::path::Path;
use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
}

// 학습에 쓰지 않은 글꼴 포함 (없으면 GDI 가 비슷한 글꼴로 대체)
const FONTS: [&str; 12] = [
    "맑은 고딕", "굴림", "돋움", "바탕", "Segoe UI", "Arial", "Tahoma", "Bahnschrift", "Corbel", "Franklin Gothic Medium", "Palatino Linotype",
    "Segoe UI Semibold",
];

const FILLER: [&str; 24] = [
    "담당자", "연락처는", "아래와", "같습니다", "서버", "접속", "정보", "관리", "대상", "장비", "목록", "비고", "확인", "요청", "결과",
    "현황", "회의", "자료", "참고", "주소", "번호", "카드", "결제", "등록",
];

fn digits(r: &mut Rng, n: usize) -> String {
    (0..n).map(|_| char::from(b'0' + r.below(10) as u8)).collect()
}

fn luhn_card(r: &mut Rng) -> String {
    loop {
        let d = format!("4{}", digits(r, 15));
        let sum: u32 = d.bytes().rev().enumerate().map(|(i, b)| {
            let mut n = (b - b'0') as u32;
            if i % 2 == 1 {
                n *= 2;
                if n > 9 {
                    n -= 9;
                }
            }
            n
        }).sum();
        if sum % 10 == 0 {
            let sep = *r.pick(&["-", " "]);
            return [&d[0..4], &d[4..8], &d[8..12], &d[12..16]].join(sep);
        }
    }
}

/// (종류, 값) — 모두 검사 규칙상 유효한 값
fn pii(r: &mut Rng) -> (&'static str, String) {
    match r.below(6) {
        0 => {
            let sep = *r.pick(&["-", "-", "."]);
            ("phone", format!("010{sep}{}{}{sep}{}", 1 + r.below(9), digits(r, 3), digits(r, 4)))
        }
        1 => ("rrn", format!("{:02}{:02}{:02}-{}{}", 60 + r.below(40), 1 + r.below(12), 1 + r.below(28), 1 + r.below(2), digits(r, 6))),
        2 => ("card", luhn_card(r)),
        3 => ("ip", format!("{}.{}.{}.{}", *r.pick(&[10, 172, 192, 203, 61, 114, 175]), r.below(256), r.below(256), 1 + r.below(254))),
        4 => {
            let user: String = (0..4 + r.below(6)).map(|_| char::from(b'a' + r.below(26) as u8)).collect();
            let n = digits(r, 2);
            ("email", format!("{user}{n}@{}.{}", r.pick(&["naver", "gmail", "korea", "daum", "mois"]), r.pick(&["com", "net", "go.kr", "co.kr"])))
        }
        _ => {
            let hex = b"0123456789ABCDEF";
            let h = |r: &mut Rng| format!("{}{}", char::from(*r.pick(hex)), char::from(*r.pick(hex)));
            let parts: Vec<String> = (0..6).map(|_| h(r)).collect();
            ("mac", format!("{}", parts.join(":")).replacen("00", "0A", 1))
        }
    }
}

/// 오탐 함정 (검출되면 안 됨)
fn decoy(r: &mut Rng) -> String {
    match r.below(6) {
        0 => format!("2024.{:02}.{:02}", 1 + r.below(12), 1 + r.below(28)),
        1 => format!("v{}.{}.{}", r.below(5), r.below(20), r.below(100)),
        2 => format!("{},{:03},000원", 1 + r.below(99), r.below(1000)),
        3 => format!("{:02}:{:02}", r.below(24), r.below(60)),
        4 => format!("p.{}", 1 + r.below(300)),
        _ => format!("{}%", r.below(100)),
    }
}

struct Dc {
    dc: HDC,
    bits: *mut u32,
}

fn gen(dir: &Path, n: usize) {
    std::fs::create_dir_all(dir).unwrap();
    let mut r = Rng(0xD1B5_4A32_D192_ED03); // 학습(0x2545…)과 다른 씨앗
    for i in 0..n {
        let (w, h) = (900 + r.below(500) as i32, 500 + r.below(500) as i32);
        let mut truth = String::new();
        let img = unsafe {
            let dc = CreateCompatibleDC(None);
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
                ..Default::default()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let old = SelectObject(dc, bmp);
            let d = Dc { dc, bits: bits as *mut u32 };
            let px = std::slice::from_raw_parts_mut(d.bits, (w * h) as usize);
            let bgv = *r.pick(&[0xFFFFFFu32, 0xF7F7F7, 0xFFFDF5, 0xEEF3FA]);
            px.fill(bgv);
            SetBkMode(d.dc, TRANSPARENT);
            let mut y = 12;
            let size = 11 + r.below(12) as i32;
            let face = *r.pick(&FONTS);
            let quality = *r.pick(&[CLEARTYPE_QUALITY, ANTIALIASED_QUALITY, NONANTIALIASED_QUALITY]);
            while y + size * 2 < h {
                // 줄 종류: 보통 / 진한 머리띠(흰 글자) / 표 선
                let kind = r.below(10);
                let (fg, bar) = if kind == 0 { (0xFFFFFFu32, Some(*r.pick(&[0x7A3D1Fu32, 0x30302C, 0x1F4E79]))) } else { (*r.pick(&[0x000000u32, 0x333333, 0x1F1F5F]), None) };
                if let Some(c) = bar {
                    let br = CreateSolidBrush(COLORREF(c));
                    let rc = RECT { left: 0, top: y - 3, right: w, bottom: y + size + 6 };
                    FillRect(d.dc, &rc, br);
                    let _ = DeleteObject(br);
                }
                let mut lf = LOGFONTW { lfHeight: -size, lfWeight: if r.below(6) == 0 { 700 } else { 400 }, lfQuality: quality, lfCharSet: DEFAULT_CHARSET, ..Default::default() };
                for (dd, s) in lf.lfFaceName.iter_mut().zip(face.encode_utf16().take(31)) {
                    *dd = s;
                }
                let font = CreateFontIndirectW(&lf);
                let of = SelectObject(d.dc, font);
                SetTextColor(d.dc, COLORREF(fg));
                let mut x = 10 + r.below(30) as i32;
                let tokens = 3 + r.below(6);
                for _ in 0..tokens {
                    let (k, t) = match r.below(10) {
                        0..=3 => ("", FILLER[r.below(FILLER.len() as u64) as usize].to_string()),
                        4..=5 => ("", decoy(&mut r)),
                        _ => {
                            let (k, v) = pii(&mut r);
                            (k, v)
                        }
                    };
                    let w16: Vec<u16> = t.encode_utf16().collect();
                    let mut sz = SIZE::default();
                    let _ = GetTextExtentPoint32W(d.dc, &w16, &mut sz);
                    if x + sz.cx > w - 10 {
                        break;
                    }
                    let _ = TextOutW(d.dc, x, y, &w16);
                    if !k.is_empty() {
                        truth += &format!("{} {} {} {} {k} {t}\n", x, y, x + sz.cx, y + sz.cy);
                    }
                    let mut sp = SIZE::default();
                    let _ = GetTextExtentPoint32W(d.dc, &[b' ' as u16, b' ' as u16], &mut sp);
                    x += sz.cx + sp.cx;
                }
                SelectObject(d.dc, of);
                let _ = DeleteObject(font);
                if kind == 1 {
                    // 표 가로선
                    let pen = CreatePen(PS_SOLID, 1, COLORREF(0x999999));
                    let op = SelectObject(d.dc, pen);
                    let _ = MoveToEx(d.dc, 0, y + size + 4, None);
                    let _ = LineTo(d.dc, w, y + size + 4);
                    SelectObject(d.dc, op);
                    let _ = DeleteObject(pen);
                }
                y += size + 8 + r.below(10) as i32;
            }
            let _ = GdiFlush();
            let img = RgbaImage::from_fn(w as u32, h as u32, |x, yy| {
                let v = px[(yy as i32 * w + x as i32) as usize];
                image::Rgba([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255])
            });
            SelectObject(d.dc, old);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(d.dc);
            img
        };
        let stem = dir.join(format!("b{i:03}"));
        if r.below(3) == 0 {
            image::DynamicImage::ImageRgba8(img).to_rgb8().save_with_format(stem.with_extension("png"), image::ImageFormat::Png).unwrap();
        } else {
            let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
            let mut f = std::fs::File::create(stem.with_extension("jpg")).unwrap();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut f, 70 + r.below(25) as u8).encode_image(&rgb).unwrap();
        }
        std::fs::write(stem.with_extension("txt"), truth).unwrap();
    }
}

fn run(dir: &Path, engine: &str) {
    let mut o = Options::default();
    o.ocr_lite = engine == "lite";
    let det = Detector::new(o);
    let (mut total, mut cover, mut exact, mut fp) = (0usize, 0usize, 0usize, 0usize);
    let mut per: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    let mut secs = 0.0;
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map_or(false, |e| e == "txt")).collect();
    files.sort();
    let verbose = std::env::var("V").is_ok();
    for t in files {
        let img_path = ["png", "jpg"].iter().map(|e| t.with_extension(e)).find(|p| p.exists()).unwrap();
        let img = image::open(&img_path).unwrap().to_rgba8();
        let truth: Vec<(f32, f32, f32, f32, String, String)> = std::fs::read_to_string(&t)
            .unwrap()
            .lines()
            .filter_map(|l| {
                let p: Vec<&str> = l.splitn(6, ' ').collect();
                (p.len() == 6).then(|| (p[0].parse().unwrap(), p[1].parse().unwrap(), p[2].parse().unwrap(), p[3].parse().unwrap(), p[4].to_string(), p[5].to_string()))
            })
            .collect();
        let st = std::time::Instant::now();
        let hits = raster::detect(&img, &det).unwrap();
        secs += st.elapsed().as_secs_f64();
        let mut used = vec![false; hits.len()];
        for (x0, y0, x1, y1, k, v) in &truth {
            total += 1;
            let e = per.entry(k.clone()).or_default();
            e.1 += 1;
            // 흐림 상자가 정답 상자 가로의 90% 이상을 덮고 세로로 겹치면 덮음
            let pick = hits.iter().enumerate().position(|(i, h)| {
                let r = h.rect;
                let pad = (r.h * 0.5).max(3.0); // blur_rect 의 가로 여유와 같게
                let ov = ((r.x + r.w + pad).min(*x1) - (r.x - pad).max(*x0)).max(0.0);
                !used[i] && ov >= 0.9 * (x1 - x0) && r.y < *y1 && r.y + r.h > *y0
            });
            match pick {
                Some(i) => {
                    used[i] = true;
                    cover += 1;
                    e.0 += 1;
                    let same = hits[i].record.original.to_ascii_lowercase() == v.to_ascii_lowercase();
                    exact += same as usize;
                    if verbose && !same {
                        println!("  값다름 {}: {v} → {}", img_path.display(), hits[i].record.original);
                    }
                }
                None => {
                    if verbose {
                        println!("  놓침 {}: {k} {v}", img_path.display());
                    }
                }
            }
        }
        // 정답과 겹치지 않는 검출 = 오탐
        for (i, h) in hits.iter().enumerate() {
            if used[i] {
                continue;
            }
            let r = h.rect;
            let on_truth = truth.iter().any(|(x0, y0, x1, y1, ..)| r.x < *x1 && r.x + r.w > *x0 && r.y < *y1 && r.y + r.h > *y0);
            if !on_truth {
                fp += 1;
                if verbose {
                    println!("  오탐 {}: {} ({})", img_path.display(), h.record.original, h.record.label());
                }
            }
        }
    }
    println!(
        "{engine}: 정답 {total}, 덮음률 {:.3}, 값정확 {:.3}, 오탐 {fp}, 검출시간 {secs:.1}초",
        cover as f32 / total as f32,
        exact as f32 / total as f32
    );
    for (k, (c, n)) in per {
        println!("   {k:<6} {c}/{n} = {:.3}", c as f32 / n as f32);
    }
}

pub fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a[0].as_str() {
        "gen" => gen(Path::new(&a[1]), a.get(2).and_then(|v| v.parse().ok()).unwrap_or(60)),
        _ => run(Path::new(&a[1]), a.get(2).map_or("lite", |s| s.as_str())),
    }
}

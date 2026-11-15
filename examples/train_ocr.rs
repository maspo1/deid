//! 자체 경량 OCR 학습 (MNIST 처럼 글자 하나씩) — 외부 데이터·네트워크 없이 이 PC 의 Windows 기본 글꼴로 합성.
//!
//! `cargo run --release --example train_ocr` → `src/ocr/glyph_model.bin`
//!
//! 1. 합성: IP·전화·카드·주민번호·이메일·MAC·영문 단어, 그리고 한글 단어("그 밖")를 여러 글꼴·크기·굵기·렌더링
//!    (ClearType/안티앨리어싱/없음)·색으로 그림
//! 2. 손상: 배율 흔들기 + JPEG 압축 (화면 캡처·문서 속 그림과 같은 조건)
//! 3. 분할: 추론과 같은 `deid::ocr::glyph::segment` — 조각 수가 글자 수와 같은 표본만 사용 (정답 정렬).
//!    한글·기호만으로 된 단어는 조각 수와 상관없이 모든 조각을 "그 밖"으로 (한글 조각이 숫자로 읽히지 않게)
//! 4. 학습: 은닉층 1개 MLP, Adam, 소프트맥스 교차 엔트로피. 검증 정확도 출력
#![cfg(windows)]

use deid::ocr::glyph::{class_of, segment, Mlp, CLASSES, FEAT};
use image::RgbaImage;
use windows::core::PCWSTR;
use windows::Win32::Foundation::SIZE;
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
    fn f(&mut self) -> f32 {
        (self.next() >> 11) as f32 / (1u64 << 53) as f32
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
}

const FONTS: [&str; 18] = [
    "Segoe UI", "맑은 고딕", "Arial", "Tahoma", "Verdana", "Consolas", "Calibri", "굴림", "돋움", "Courier New", "Times New Roman",
    "Lucida Console", "Microsoft Sans Serif", "Cambria", "Candara", "Trebuchet MS", "Georgia", "바탕",
];

/// 자주 쓰이는 한글 음절 (조각 모양을 고루 담도록 받침 있는 글자 포함)
const HANGUL: &str = "가나다라마바사아자차카타파하이의는을를에서로고도만한국민정부행안전업무보고회의서버접속주소번호전화연락처카드결제주민등록외국인생년월일담당자이름확인요청처리완료예정일정기간작성수정삭제관리시스템네트워크보안개인정보통신망장비설정값비밀암호세션쿠키인증키발급대상현황결과분석계획추진과제내용항목구분비고합계총건수율및또는위아래왼쪽오른쪽없음있음";

/// 합성 단어 (공백 없음)
fn word(r: &mut Rng) -> String {
    let octet = |r: &mut Rng| match r.below(3) {
        0 => r.below(10).to_string(),
        1 => (10 + r.below(90)).to_string(),
        _ => (100 + r.below(156)).to_string(),
    };
    let digits = |r: &mut Rng, n: u64| (0..n).map(|_| char::from(b'0' + r.below(10) as u8)).collect::<String>();
    let lower = b"abcdefghijklmnopqrstuvwxyz";
    let letters = |r: &mut Rng, n: u64| {
        (0..n).map(|_| {
            let c = char::from(*r.pick(lower));
            if r.below(4) == 0 { c.to_ascii_uppercase() } else { c }
        }).collect::<String>()
    };
    let hangul: Vec<char> = HANGUL.chars().collect();
    match r.below(16) {
        0..=3 => (0..4).map(|_| octet(r)).collect::<Vec<_>>().join("."),
        4 => {
            let mid = 3 + r.below(2);
            let sep = *r.pick(&["-", ".", ""]);
            format!("0{}{sep}{}{sep}{}", digits(r, 2), digits(r, mid), digits(r, 4))
        }
        5 => format!("{}:{}", digits(r, 2), digits(r, 2)) + if r.below(2) == 0 { "" } else { ":59" },
        6 => {
            let n = 4 + r.below(9);
            digits(r, n)
        }
        7 => format!("{}-{}", digits(r, 6), digits(r, 7)),
        8 => (0..4).map(|_| digits(r, 4)).collect::<Vec<_>>().join("-"),
        9 => {
            // 이메일
            let user_len = 3 + r.below(8);
            let mut u = letters(r, user_len);
            if r.below(2) == 0 {
                u.push(*r.pick(&['.', '_', '-']));
                let k = 1 + r.below(4);
                u += &digits(r, k);
            }
            let dom_len = 3 + r.below(6);
            format!("{u}@{}.{}", letters(r, dom_len), r.pick(&["com", "net", "kr", "co.kr", "org", "go.kr"]))
        }
        10 => {
            // MAC
            let hex = b"0123456789abcdefABCDEF";
            let sep = *r.pick(&[":", "-"]);
            (0..6).map(|_| format!("{}{}", char::from(*r.pick(hex)), char::from(*r.pick(hex)))).collect::<Vec<_>>().join(sep)
        }
        11 | 12 => {
            // 영문 단어 (숫자·괄호·슬래시·쉼표 섞음)
            let n = 2 + r.below(8);
            let mut w = letters(r, n);
            match r.below(5) {
                0 => w = format!("({w})"),
                1 => w = format!("{w}/{}", digits(r, 2)),
                2 => w.push(','),
                3 => w = format!("{w}_{}", letters(r, 2)),
                _ => {}
            }
            w
        }
        13 | 14 => {
            // 한글 → 모든 조각 "그 밖"
            let n = 1 + r.below(5);
            (0..n).map(|_| *r.pick(&hangul)).collect()
        }
        _ => {
            // 목록에 없는 기호 ("그 밖")
            let sym = ['#', '%', '&', '*', '[', ']', '+', '=', '~', '<', '>', '!', '?', '"', '\'', ';', '|', '{', '}'];
            let n = 1 + r.below(4);
            (0..n).map(|_| *r.pick(&sym)).collect()
        }
    }
}

/// 모든 글자가 "그 밖"(한글·목록 밖 기호)인 단어
fn all_other(text: &str) -> bool {
    text.chars().all(|c| class_of(c) == CLASSES.len() - 1)
}

/// GDI 로 단어 그리기 → (그림, 글자 높이 px)
fn render(r: &mut Rng, text: &str) -> Option<RgbaImage> {
    let size = 9 + r.below(16) as i32; // 글자 셀 높이 9~24px
    let face = r.pick(&FONTS);
    let quality = *r.pick(&[CLEARTYPE_QUALITY, ANTIALIASED_QUALITY, NONANTIALIASED_QUALITY, CLEARTYPE_QUALITY]);
    let bold = r.below(5) == 0;
    // 색: 대부분 진한 글자/밝은 바탕, 일부 밝은 글자/어두운 바탕
    let (fg, bg) = if r.below(5) == 0 {
        ([230 + r.below(26) as u8; 3], [r.below(60) as u8, r.below(60) as u8, 40 + r.below(120) as u8])
    } else {
        let g = r.below(70) as u8;
        ([g, g, g.saturating_add(r.below(40) as u8)], [200 + r.below(56) as u8, 200 + r.below(56) as u8, 200 + r.below(56) as u8])
    };
    let w16: Vec<u16> = text.encode_utf16().collect();
    unsafe {
        let dc = CreateCompatibleDC(None);
        let mut lf = LOGFONTW { lfHeight: -size, lfWeight: if bold { 700 } else { 400 }, lfQuality: quality, lfCharSet: DEFAULT_CHARSET, ..Default::default() };
        for (d, s) in lf.lfFaceName.iter_mut().zip(face.encode_utf16().take(31)) {
            *d = s;
        }
        let font = CreateFontIndirectW(&lf);
        let old_font = SelectObject(dc, font);
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &w16, &mut sz);
        let (w, h) = (sz.cx + 8, sz.cy + 6);
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(dc, &bi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old_bmp = SelectObject(dc, bmp);
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
        let bgv = (bg[0] as u32) << 16 | (bg[1] as u32) << 8 | bg[2] as u32;
        px.fill(bgv);
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(fg[0] as u32 | (fg[1] as u32) << 8 | (fg[2] as u32) << 16));
        let _ = TextOutW(dc, 4, 3, &w16);
        let _ = GdiFlush();
        let img = RgbaImage::from_fn(w as u32, h as u32, |x, y| {
            let v = px[(y as i32 * w + x as i32) as usize];
            image::Rgba([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255])
        });
        SelectObject(dc, old_bmp);
        SelectObject(dc, old_font);
        let _ = DeleteObject(bmp);
        let _ = DeleteObject(font);
        let _ = DeleteDC(dc);
        let _ = PCWSTR::null();
        Some(img)
    }
}

/// 글자가 있는 부분 + 무작위 여백만 남김 (OCR 단어 상자와 비슷하게)
fn crop_ink(r: &mut Rng, img: &RgbaImage) -> Option<RgbaImage> {
    let bgp = *img.get_pixel(0, 0);
    let diff = |p: &image::Rgba<u8>| (0..3).map(|c| (p[c] as i32 - bgp[c] as i32).abs()).max().unwrap_or(0) > 40;
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if diff(p) {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 > x1 {
        return None;
    }
    let pad = |r: &mut Rng| r.below(3) as u32;
    let (a, b) = (x0.saturating_sub(pad(r)), y0.saturating_sub(pad(r)));
    let (c, d) = ((x1 + 1 + pad(r)).min(img.width()), (y1 + 1 + pad(r)).min(img.height()));
    Some(image::imageops::crop_imm(img, a, b, c - a, d - b).to_image())
}

/// 배율 흔들기 + JPEG
fn degrade(r: &mut Rng, img: RgbaImage) -> RgbaImage {
    let mut img = img;
    if r.below(2) == 0 {
        let k = 0.75 + 0.55 * r.f();
        let (w, h) = (((img.width() as f32 * k) as u32).max(2), ((img.height() as f32 * k) as u32).max(4));
        img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    }
    if r.below(3) != 0 {
        let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
        let mut buf = Vec::new();
        let q = 55 + r.below(41) as u8;
        if image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q).encode_image(&rgb).is_ok() {
            if let Ok(d) = image::load_from_memory(&buf) {
                img = d.to_rgba8();
            }
        }
    }
    img
}

fn main() {
    let mut r = Rng(0x2545_F491_4F6C_DD1D);
    let target: usize = std::env::args().nth(1).and_then(|v| v.parse().ok()).unwrap_or(500_000);
    let (mut xs, mut ys): (Vec<[f32; FEAT]>, Vec<usize>) = (Vec::new(), Vec::new());
    let (mut tried, mut kept) = (0, 0);
    let t = std::time::Instant::now();
    while xs.len() < target {
        tried += 1;
        let text = word(&mut r);
        let Some(img) = render(&mut r, &text) else { continue };
        let Some(img) = crop_ink(&mut r, &img) else { continue };
        let img = degrade(&mut r, img);
        let gl = segment(&img);
        if all_other(&text) {
            // 한글·기호: 조각 수와 상관없이 전부 "그 밖"
            kept += 1;
            for g in &gl {
                xs.push(g.feat);
                ys.push(CLASSES.len() - 1);
            }
            continue;
        }
        if gl.len() != text.chars().count() {
            continue; // 분할이 글자 수와 다르면 정답을 맞출 수 없으므로 버림
        }
        kept += 1;
        for (g, c) in gl.iter().zip(text.chars()) {
            xs.push(g.feat);
            ys.push(class_of(c));
        }
    }
    let mut counts = [0usize; CLASSES.len()];
    for &y in &ys {
        counts[y] += 1;
    }
    println!("합성: 단어 {tried}개 중 분할 일치 {kept}개 ({:.1}%), 글자 {}개, {:.0}초", 100.0 * kept as f32 / tried as f32, xs.len(), t.elapsed().as_secs_f32());
    println!("클래스별: {}", CLASSES.iter().zip(counts).map(|(c, n)| format!("{c}:{n}")).collect::<Vec<_>>().join(" "));

    // 학습/검증 분할 (10%)
    let n = xs.len();
    let mut order: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        order.swap(i, r.below(i as u64 + 1) as usize);
    }
    let (val, train) = order.split_at(n / 10);
    let hidden: usize = std::env::var("HIDDEN").ok().and_then(|v| v.parse().ok()).unwrap_or(128);
    let k = CLASSES.len();
    let init = |r: &mut Rng, fan_in: usize| (r.f() * 2.0 - 1.0) * (6.0 / fan_in as f32).sqrt();
    let mut m = Mlp {
        hidden,
        w1: (0..hidden * FEAT).map(|_| init(&mut r, FEAT)).collect(),
        b1: vec![0.0; hidden],
        w2: (0..k * hidden).map(|_| init(&mut r, hidden)).collect(),
        b2: vec![0.0; k],
    };
    // Adam
    let sizes = [m.w1.len(), m.b1.len(), m.w2.len(), m.b2.len()];
    let mut mo: Vec<Vec<f32>> = sizes.iter().map(|&s| vec![0.0; s]).collect();
    let mut ve: Vec<Vec<f32>> = sizes.iter().map(|&s| vec![0.0; s]).collect();
    let (b1m, b2m, eps) = (0.9f32, 0.999f32, 1e-8f32);
    let mut step = 0i32;
    let batch = 64;
    let mut train = train.to_vec();
    for epoch in 0..12 {
        let lr = 0.002 * 0.8f32.powi(epoch);
        for i in (1..train.len()).rev() {
            train.swap(i, r.below(i as u64 + 1) as usize);
        }
        let mut loss = 0.0f32;
        for chunk in train.chunks(batch) {
            let mut g: Vec<Vec<f32>> = sizes.iter().map(|&s| vec![0.0; s]).collect();
            for &i in chunk {
                let x = &xs[i];
                let mut h = vec![0f32; hidden];
                for j in 0..hidden {
                    let z: f32 = m.w1[j * FEAT..(j + 1) * FEAT].iter().zip(x).map(|(a, b)| a * b).sum::<f32>() + m.b1[j];
                    h[j] = z.max(0.0);
                }
                let mut o = vec![0f32; k];
                for c in 0..k {
                    o[c] = m.w2[c * hidden..(c + 1) * hidden].iter().zip(&h).map(|(a, b)| a * b).sum::<f32>() + m.b2[c];
                }
                let mx = o.iter().cloned().fold(f32::MIN, f32::max);
                let s: f32 = o.iter().map(|v| (v - mx).exp()).sum();
                let p: Vec<f32> = o.iter().map(|v| (v - mx).exp() / s).collect();
                loss -= p[ys[i]].max(1e-9).ln();
                // 역전파
                let mut dh = vec![0f32; hidden];
                for c in 0..k {
                    let d = p[c] - (c == ys[i]) as u8 as f32;
                    g[3][c] += d;
                    for j in 0..hidden {
                        g[2][c * hidden + j] += d * h[j];
                        dh[j] += d * m.w2[c * hidden + j];
                    }
                }
                for j in 0..hidden {
                    if h[j] <= 0.0 {
                        continue;
                    }
                    g[1][j] += dh[j];
                    let row = &mut g[0][j * FEAT..(j + 1) * FEAT];
                    for (gv, xv) in row.iter_mut().zip(x) {
                        *gv += dh[j] * xv;
                    }
                }
            }
            step += 1;
            let (c1, c2) = (1.0 - b1m.powi(step), 1.0 - b2m.powi(step));
            let bs = chunk.len() as f32;
            for (pi, params) in [&mut m.w1, &mut m.b1, &mut m.w2, &mut m.b2].into_iter().enumerate() {
                for (q, w) in params.iter_mut().enumerate() {
                    let gr = g[pi][q] / bs;
                    mo[pi][q] = b1m * mo[pi][q] + (1.0 - b1m) * gr;
                    ve[pi][q] = b2m * ve[pi][q] + (1.0 - b2m) * gr * gr;
                    *w -= lr * (mo[pi][q] / c1) / ((ve[pi][q] / c2).sqrt() + eps);
                }
            }
        }
        let acc = val.iter().filter(|&&i| {
            let p = m.predict(&xs[i]);
            p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|v| v.0) == Some(ys[i])
        }).count() as f32
            / val.len() as f32;
        println!("에폭 {epoch:>2}: 학습 손실 {:.4}, 검증 정확도 {:.4} ({:.0}초)", loss / train.len() as f32, acc, t.elapsed().as_secs_f32());
    }
    // 클래스별 검증 정확도
    let mut per = vec![(0usize, 0usize); k];
    for &i in val {
        let p = m.predict(&xs[i]);
        let a = p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|v| v.0).unwrap();
        per[ys[i]].1 += 1;
        per[ys[i]].0 += (a == ys[i]) as usize;
    }
    println!("클래스별 검증: {}", CLASSES.iter().zip(&per).map(|(c, (ok, n))| format!("{c}:{:.3}", *ok as f32 / (*n).max(1) as f32)).collect::<Vec<_>>().join(" "));
    let out = concat!(env!("CARGO_MANIFEST_DIR"), "/src/ocr/glyph_model.bin");
    std::fs::write(out, m.to_bytes()).expect("저장");
    println!("저장: {out} ({} 바이트)", m.to_bytes().len());
}

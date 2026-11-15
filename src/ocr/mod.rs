//! 이미지 속 글자 인식 — OS 내장 OCR 사용 (exe 외 추가 파일 불필요).
//!
//! * Windows 10/11 : `Windows.Media.Ocr` (한국어 OCR 언어팩: 한국어 Windows 기본 포함) — `windows.rs`
//! * macOS 10.15+  : Vision 프레임워크 `VNRecognizeTextRequest` (한국어는 macOS 13+) — `macos.rs`
//! * 그 밖의 OS    : OCR 없음 (그림 검사는 메타데이터만)
//!
//! 플랫폼 모듈은 두 함수만 제공한다: `max_dimension()`, `recognize(&RgbaImage) -> 단어 좌표(픽셀, 왼쪽 위 원점)`.
//!
//! 작은 글자 인식률을 높이기 위해 여러 배율로 확대해 인식하고, 좌표는 원본 기준으로 되돌린다.

use image::RgbaImage;

pub mod glyph;
pub mod lite;

/// 인식된 단어 (원본 이미지 좌표)
#[derive(Clone, Debug)]
pub struct OcrWord {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// 한 줄 = 단어 목록
#[derive(Clone, Debug, Default)]
pub struct OcrLine {
    pub words: Vec<OcrWord>,
}

/// 투명 픽셀을 흰 배경에 합성 (투명 영역이 검게 인식되는 문제 방지)
fn flatten_on_white(img: &RgbaImage) -> RgbaImage {
    let mut out = img.clone();
    for p in out.pixels_mut() {
        let a = p[3] as u32;
        if a < 255 {
            for c in 0..3 {
                p[c] = ((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
    }
    out
}

/// 흑백 이진화 (오츠 임계값) — 화면 캡처의 옅은 바탕·안티앨리어싱을 걷어내 작은 글자와 점(.)이 또렷해진다.
/// 좌표는 바뀌지 않는다 (같은 크기).
pub fn enhance(img: &RgbaImage) -> RgbaImage {
    let lum = |p: &image::Rgba<u8>| ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as usize;
    let mut hist = [0u64; 256];
    for p in img.pixels() {
        hist[lum(p)] += 1;
    }
    let total = img.pixels().len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut wb, mut sb, mut best, mut th) = (0f64, 0f64, -1f64, 128usize);
    for (t, &c) in hist.iter().enumerate() {
        wb += c as f64;
        if wb == 0.0 || wb == total {
            continue;
        }
        sb += t as f64 * c as f64;
        let wf = total - wb;
        let v = wb * wf * (sb / wb - (sum - sb) / wf).powi(2);
        if v > best {
            best = v;
            th = t;
        }
    }
    let mut out = img.clone();
    for p in out.pixels_mut() {
        let v = if lum(p) > th { 255 } else { 0 };
        *p = image::Rgba([v, v, v, 255]);
    }
    out
}

/// 글자보다 굵고 긴 세로 막대(그래프·표 선) 지우기 — 어두운 세로 연속 14px 이상이 가로 3px 이상 이어지면 흰색.
/// 화면 캡처에서 글자 옆 막대를 OCR 이 `1` 로 읽는 것을 막는다. 큰 글씨 세로획도 지울 수 있어 최고 심층 단계에서만 쓴다.
pub fn remove_bars(img: &RgbaImage) -> RgbaImage {
    let lum = |p: &image::Rgba<u8>| (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
    let (w, h) = img.dimensions();
    let mut run = vec![0u32; (w as usize) * (h as usize)];
    for x in 0..w {
        let mut y = 0;
        while y < h {
            if lum(img.get_pixel(x, y)) >= 160 {
                y += 1;
                continue;
            }
            let s = y;
            while y < h && lum(img.get_pixel(x, y)) < 160 {
                y += 1;
            }
            for yy in s..y {
                run[(yy * w + x) as usize] = y - s;
            }
        }
    }
    let mut out = img.clone();
    for y in 0..h {
        let tall = |x: u32| run[(y * w + x) as usize] >= 14;
        for x in 0..w {
            let three = (x >= 2 && tall(x - 1) && tall(x - 2)) || (x >= 1 && x + 1 < w && tall(x - 1) && tall(x + 1)) || (x + 2 < w && tall(x + 1) && tall(x + 2));
            if tall(x) && three {
                out.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
    }
    out
}

/// 실제 적용 배율: 요청 배율을 OCR 엔진 한계(긴 변)와 `max_pixels`(확대 후 화소 수 상한)로 제한.
/// 원본이 이미 상한보다 크면 축소하지 않고 원본 그대로(배율 1) 인식한다.
pub fn effective_scale(img: &RgbaImage, scale: f32, max_pixels: u64) -> Result<f32, String> {
    let max_dim = platform::max_dimension()? as f32;
    let longest = img.width().max(img.height()) as f32;
    let px = (img.width() as f64 * img.height() as f64).max(1.0);
    let by_pixels = (max_pixels as f64 / px).sqrt() as f32;
    Ok(scale.min(max_dim / longest).min(by_pixels).max(1.0f32.min(scale)))
}

/// 배율 `s` 로 확대해 인식. 반환 좌표는 원본 기준.
pub fn recognize_scaled(img: &RgbaImage, s: f32) -> Result<Vec<OcrLine>, String> {
    let max_dim = platform::max_dimension()? as f32;
    let longest = img.width().max(img.height()) as f32;
    let s = s.min(max_dim / longest).max(0.1);
    let base = flatten_on_white(img);
    let scaled = if (s - 1.0).abs() < 0.01 {
        base
    } else {
        let (w, h) = ((img.width() as f32 * s).round() as u32, (img.height() as f32 * s).round() as u32);
        image::imageops::resize(&base, w.max(1), h.max(1), image::imageops::FilterType::CatmullRom)
    };
    let mut lines = platform::recognize(&scaled)?;
    for l in &mut lines {
        for w in &mut l.words {
            w.x /= s;
            w.y /= s;
            w.w /= s;
            w.h /= s;
        }
    }
    Ok(lines)
}

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::OcrLine;
    use image::RgbaImage;
    pub fn max_dimension() -> Result<u32, String> {
        Err("이 OS 에서는 내장 OCR 을 지원하지 않습니다 (Windows 10/11, macOS 지원)".into())
    }
    pub fn recognize(_: &RgbaImage) -> Result<Vec<OcrLine>, String> {
        max_dimension().map(|_| vec![])
    }
}

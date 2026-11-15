//! 자체 학습 경량 OCR — 글자 하나씩 분류하는 작은 신경망 (MNIST 방식).
//!
//! 쓰임 두 가지:
//! * `lite` 엔진(기본): 줄 그림을 이 모듈로 글자 단위로 읽는다.
//! * 최고 심층 단계: Windows OCR 이 찾은 숫자 단어를 다시 읽는다 (`reread`).
//!
//! 흐름: 단어/줄 그림 → 회색조 → 높이 48px 로 확대 → 오츠 이진화 → 연결 성분 →
//!       가로로 겹치는 성분은 한 글자(`:` `i`) → 글자마다 12×16 비트맵 + 위치·크기 4개 특징 → MLP 분류
//! 분류: 정보 검사에 쓰이는 글자만 — `0-9`, `a-z`(대소문자 합침), `. - : @ _ / ( ) ,`, "그 밖"(?: 한글·기호).
//! 한글은 읽지 않고 "그 밖"으로 분류한다 (한글 조각이 숫자로 읽혀 오탐이 되지 않도록 학습).
//!
//! 학습(`examples/train_ocr`)과 추론이 이 파일의 같은 함수를 쓴다. 가중치는 `glyph_model.bin` 을 exe 에 내장.

use image::{GrayImage, Luma, RgbaImage};

pub const CLASSES: [char; 46] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p',
    'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '.', '-', ':', '@', '_', '/', '(', ')', ',', '?',
];

/// 글자 → 클래스 번호 (대문자는 소문자 클래스, 목록에 없는 글자는 '?')
pub fn class_of(c: char) -> usize {
    let c = c.to_ascii_lowercase();
    CLASSES.iter().position(|&k| k == c).unwrap_or(CLASSES.len() - 1)
}
pub const BW: usize = 12;
pub const BH: usize = 16;
pub const FEAT: usize = BW * BH + 4;
/// 확대 후 단어 높이
const NORM_H: u32 = 48;

pub struct Glyph {
    /// 확대한 그림에서의 가로 범위
    pub x0: u32,
    pub x1: u32,
    pub feat: [f32; FEAT],
}

/// 단어 그림 → 글자 조각 (왼쪽부터)
pub fn segment(word: &RgbaImage) -> Vec<Glyph> {
    let (w0, h0) = word.dimensions();
    if w0 < 2 || h0 < 4 {
        return vec![];
    }
    let gray = GrayImage::from_fn(w0, h0, |x, y| {
        let p = word.get_pixel(x, y);
        Luma([((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000) as u8])
    });
    segment_gray(&gray, None, None).0
}

/// 회색조 그림 → (글자 조각, 확대 배율). `light_text` 를 알면 극성을 정하고, 모르면 다수결로 정한다.
/// 줄 그림(`lite`)에서는 위아래 가장자리에 닿고 가운데 띠와 거의 겹치지 않는 성분(이웃 줄 조각)을 버린다.
pub fn segment_gray(gray: &GrayImage, light_text: Option<bool>, accept: Option<&dyn Fn(&[f32; FEAT]) -> bool>) -> (Vec<Glyph>, f32) {
    let (w0, h0) = gray.dimensions();
    if w0 < 2 || h0 < 4 {
        return (vec![], 1.0);
    }
    let k = NORM_H as f32 / h0 as f32;
    let (w, h) = (((w0 as f32 * k).round() as u32).clamp(1, 8000), NORM_H);
    let big = image::imageops::resize(gray, w, h, image::imageops::FilterType::CatmullRom);
    let th = otsu(&big);
    let dark = big.pixels().filter(|p| p[0] as u32 <= th).count();
    // 글자는 소수 — 어두운 쪽이 절반을 넘으면 밝은 글자
    let light_text_known = light_text.is_some();
    let light_text = light_text.unwrap_or(dark * 2 > (w * h) as usize);
    let ink: Vec<bool> = big.pixels().map(|p| (p[0] as u32 <= th) != light_text).collect();

    // 연결 성분 (8방향)
    let idx = |x: u32, y: u32| (y * w + x) as usize;
    let mut label = vec![u32::MAX; ink.len()];
    let mut comps: Vec<(u32, u32, u32, u32, u32)> = Vec::new(); // x0,y0,x1,y1,area
    let mut stack = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if !ink[idx(x, y)] || label[idx(x, y)] != u32::MAX {
                continue;
            }
            let id = comps.len() as u32;
            let mut c = (x, y, x, y, 0);
            label[idx(x, y)] = id;
            stack.push((x, y));
            while let Some((cx, cy)) = stack.pop() {
                c = (c.0.min(cx), c.1.min(cy), c.2.max(cx), c.3.max(cy), c.4 + 1);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let i = idx(nx as u32, ny as u32);
                        if ink[i] && label[i] == u32::MAX {
                            label[i] = id;
                            stack.push((nx as u32, ny as u32));
                        }
                    }
                }
            }
            comps.push(c);
        }
    }
    // 잡티 제거, 가로로 겹치는 성분은 한 글자로 (콜론의 두 점)
    let mut comps: Vec<(u32, u32, u32, u32, u32)> = comps.into_iter().filter(|c| c.4 >= 6).collect();
    if light_text_known {
        // 이웃 줄 조각: 위/아래 끝에 닿아 있고 높이가 띠의 30% 미만
        comps.retain(|c| !((c.1 == 0 || c.3 == h - 1) && (c.3 - c.1 + 1) * 10 < h * 3 && c.2 - c.0 + 1 < h));
    }
    comps.sort_by_key(|c| c.0);
    let mut groups: Vec<(u32, u32, u32, u32)> = Vec::new();
    for c in comps {
        if let Some(g) = groups.last_mut() {
            let ov = (g.2.min(c.2) as i64 - g.0.max(c.0) as i64 + 1).max(0) as f32;
            let narrow = ((g.2 - g.0 + 1).min(c.2 - c.0 + 1)) as f32;
            if ov >= 0.5 * narrow {
                *g = (g.0.min(c.0), g.1.min(c.1), g.2.max(c.2), g.3.max(c.3));
                continue;
            }
        }
        groups.push((c.0, c.1, c.2, c.3));
    }
    let feat_of = |(x0, y0, x1, y1): (u32, u32, u32, u32)| -> [f32; FEAT] {
        let mut feat = [0f32; FEAT];
        let (gw, gh) = (x1 - x0 + 1, y1 - y0 + 1);
        // 비트맵: 비율 유지해 12×16 안에 맞춤 (가운데)
        let s = (BW as f32 / gw as f32).min(BH as f32 / gh as f32);
        let (tw, tht) = ((gw as f32 * s).max(1.0), (gh as f32 * s).max(1.0));
        let (ox, oy) = ((BW as f32 - tw) / 2.0, (BH as f32 - tht) / 2.0);
        for by in 0..BH {
            for bx in 0..BW {
                // 칸 하나를 2×2 표본으로
                let mut v = 0.0;
                for sy in 0..2 {
                    for sx in 0..2 {
                        let fx = (bx as f32 + 0.25 + 0.5 * sx as f32 - ox) / s;
                        let fy = (by as f32 + 0.25 + 0.5 * sy as f32 - oy) / s;
                        if fx >= 0.0 && fy >= 0.0 && fx < gw as f32 && fy < gh as f32 && ink[idx(x0 + fx as u32, y0 + fy as u32)] {
                            v += 0.25;
                        }
                    }
                }
                feat[by * BW + bx] = v;
            }
        }
        let hf = h as f32;
        feat[BW * BH] = y0 as f32 / hf;
        feat[BW * BH + 1] = (y1 + 1) as f32 / hf;
        feat[BW * BH + 2] = gw as f32 / hf;
        feat[BW * BH + 3] = gh as f32 / hf;
        feat
    };
    // 가로 범위 안의 잉크 상자 (없으면 None)
    let tight = |x0: u32, x1: u32, y0: u32, y1: u32| -> Option<(u32, u32, u32, u32)> {
        let (mut a, mut b, mut c, mut d) = (u32::MAX, u32::MAX, 0, 0);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if ink[idx(x, y)] {
                    a = a.min(x);
                    b = b.min(y);
                    c = c.max(x);
                    d = d.max(y);
                }
            }
        }
        (a != u32::MAX).then_some((a, b, c, d))
    };
    let mut out: Vec<Glyph> = Vec::with_capacity(groups.len());
    for g in groups {
        // 붙은 글자 나누기 (줄 판독 전용): 넓은데 확신 있게 읽히지 않으면, 가운데 근처 잉크가 가장 얇은 열에서 자른다.
        // 두 조각이 모두 확신 있게 읽힐 때만 채택 — m·w·@ 같은 넓은 글자는 그대로 남는다.
        if let Some(acc) = accept {
            // 열의 잉크 수
            let col = |x: u32, y0: u32, y1: u32| -> u32 { (y0..=y1).filter(|&y| ink[idx(x, y)]).count() as u32 };
            if let Some(parts) = split_touching(g, h, &feat_of, &tight, acc, &col, 2) {
                for p in parts {
                    // 자르고 남은 1~2px 부스러기는 버림
                    if (p.2 - p.0 + 1) * 16 < h {
                        continue;
                    }
                    out.push(Glyph { x0: p.0, x1: p.2, feat: feat_of(p) });
                }
                continue;
            }
        }
        out.push(Glyph { x0: g.0, x1: g.2, feat: feat_of(g) });
    }
    let glyphs = out;
    (glyphs, k)
}

type Rect4 = (u32, u32, u32, u32);

/// 붙은 글자를 둘로 (재귀 `depth` 번까지). 나눌 필요가 없거나 나눠도 확신이 없으면 None.
fn split_touching(
    g: Rect4,
    h: u32,
    feat_of: &dyn Fn(Rect4) -> [f32; FEAT],
    tight: &dyn Fn(u32, u32, u32, u32) -> Option<Rect4>,
    accept: &dyn Fn(&[f32; FEAT]) -> bool,
    col: &dyn Fn(u32, u32, u32) -> u32,
    depth: u8,
) -> Option<Vec<Rect4>> {
    let (x0, y0, x1, y1) = g;
    let gw = x1 - x0 + 1;
    // 숫자 한 글자는 띠 높이의 0.6배 안팎 — 0.75배 넘게 넓고 확신이 없을 때만
    if depth == 0 || gw * 4 < h * 3 || accept(&feat_of(g)) {
        return None;
    }
    let (lo, hi) = (x0 + gw / 4, x1 - gw / 4);
    let mid = (x0 + x1) / 2;
    let cut = (lo..=hi).min_by_key(|&x| (col(x, y0, y1), (x as i64 - mid as i64).abs()))?;
    let left = tight(x0, cut.saturating_sub(1).max(x0), y0, y1)?;
    let right = tight((cut + 1).min(x1), x1, y0, y1)?;
    let mut parts = Vec::new();
    for p in [left, right] {
        if accept(&feat_of(p)) {
            parts.push(p);
        } else {
            parts.extend(split_touching(p, h, feat_of, tight, accept, col, depth - 1)?);
        }
    }
    Some(parts)
}

fn otsu(img: &GrayImage) -> u32 {
    let mut hist = [0u64; 256];
    for p in img.pixels() {
        hist[p[0] as usize] += 1;
    }
    let total = img.pixels().len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut wb, mut sb, mut best, mut th) = (0f64, 0f64, -1f64, 127u32);
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
            th = t as u32;
        }
    }
    th
}

/// 은닉층 1개 MLP (ReLU, 소프트맥스) — 가중치는 행 우선 f32
pub struct Mlp {
    pub hidden: usize,
    pub w1: Vec<f32>, // hidden × FEAT
    pub b1: Vec<f32>,
    pub w2: Vec<f32>, // CLASSES × hidden
    pub b2: Vec<f32>,
}

impl Mlp {
    /// 클래스별 확률
    pub fn predict(&self, x: &[f32; FEAT]) -> [f32; CLASSES.len()] {
        let mut hdn = vec![0f32; self.hidden];
        for (j, hv) in hdn.iter_mut().enumerate() {
            let row = &self.w1[j * FEAT..(j + 1) * FEAT];
            let z: f32 = row.iter().zip(x.iter()).map(|(a, b)| a * b).sum::<f32>() + self.b1[j];
            *hv = z.max(0.0);
        }
        let mut out = [0f32; CLASSES.len()];
        for (k, o) in out.iter_mut().enumerate() {
            let row = &self.w2[k * self.hidden..(k + 1) * self.hidden];
            *o = row.iter().zip(&hdn).map(|(a, b)| a * b).sum::<f32>() + self.b2[k];
        }
        let m = out.iter().cloned().fold(f32::MIN, f32::max);
        let mut sum = 0.0;
        for o in &mut out {
            *o = (*o - m).exp();
            sum += *o;
        }
        for o in &mut out {
            *o /= sum;
        }
        out
    }

    /// 저장 형식: "DGLY" + hidden(u32) + w1,b1,w2,b2 (f32 LE)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = b"DGLY".to_vec();
        v.extend((self.hidden as u32).to_le_bytes());
        for a in [&self.w1, &self.b1, &self.w2, &self.b2] {
            for f in a.iter() {
                v.extend(f.to_le_bytes());
            }
        }
        v
    }

    pub fn from_bytes(b: &[u8]) -> Option<Mlp> {
        if b.len() < 8 || &b[..4] != b"DGLY" {
            return None;
        }
        let hidden = u32::from_le_bytes(b[4..8].try_into().ok()?) as usize;
        let n = [hidden * FEAT, hidden, CLASSES.len() * hidden, CLASSES.len()];
        if b.len() != 8 + n.iter().sum::<usize>() * 4 {
            return None;
        }
        let mut off = 8;
        let mut take = |k: usize| {
            let v: Vec<f32> = b[off..off + k * 4].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
            off += k * 4;
            v
        };
        Some(Mlp { hidden, w1: take(n[0]), b1: take(n[1]), w2: take(n[2]), b2: take(n[3]) })
    }
}

/// exe 에 내장한 학습 가중치 (없으면 None — 최고 단계에서 이 다시 읽기만 빠진다)
pub fn model() -> Option<&'static Mlp> {
    static M: std::sync::OnceLock<Option<Mlp>> = std::sync::OnceLock::new();
    M.get_or_init(|| Mlp::from_bytes(include_bytes!("glyph_model.bin"))).as_ref()
}

/// 단어 그림을 다시 읽음 → (글자열, 가장 낮은 글자 확률). 숫자 글자가 아닌 조각이 있으면 None
pub fn read_word(word: &RgbaImage) -> Option<(String, f32)> {
    let m = model()?;
    let glyphs = segment(word);
    if glyphs.is_empty() {
        return None;
    }
    let mut s = String::new();
    let mut low = 1.0f32;
    for g in &glyphs {
        let p = m.predict(&g.feat);
        let (k, &pk) = p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))?;
        if CLASSES[k] == '?' {
            return None;
        }
        s.push(CLASSES[k]);
        low = low.min(pk);
    }
    Some((s, low))
}

/// Windows OCR 이 찾은 줄에서 숫자가 2개 이상 든 단어를 이 모델로 다시 읽은 줄 (바뀐 단어가 없으면 빈 목록).
/// `img` 는 원본 좌표의 그림. 모든 글자 확률이 `min_p` 이상일 때만 바꾸고, 앞뒤의 '그 밖' 글자(아이콘 등)는 뗀다.
pub fn reread(img: &RgbaImage, lines: &[super::OcrLine], min_p: f32) -> Vec<super::OcrLine> {
    let Some(m) = model() else { return vec![] };
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let mut out = Vec::new();
    for l in lines {
        let mut nl = l.clone();
        let mut changed = false;
        for w in &mut nl.words {
            if w.text.chars().filter(|c| c.is_ascii_digit()).count() < 2 {
                continue;
            }
            let (x0, y0) = ((w.x - 1.0).max(0.0), (w.y - 1.0).max(0.0));
            let (x1, y1) = ((w.x + w.w + 1.0).min(iw), (w.y + w.h + 1.0).min(ih));
            if x1 - x0 < 2.0 || y1 - y0 < 4.0 {
                continue;
            }
            let crop = image::imageops::crop_imm(img, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32).to_image();
            let mut s = String::new();
            let mut ok = true;
            for g in segment(&crop) {
                let p = m.predict(&g.feat);
                let Some((k, &pk)) = p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)) else { continue };
                // 숫자 단어 다시 읽기 — 영문은 "그 밖"으로 본다
                let ch = if CLASSES[k].is_ascii_alphabetic() { '?' } else { CLASSES[k] };
                if ch != '?' && pk < min_p {
                    ok = false;
                    break;
                }
                s.push(ch);
            }
            let core = s.trim_matches('?');
            if ok && !core.contains('?') && core.chars().filter(|c| c.is_ascii_digit()).count() >= 2 && core != w.text {
                w.text = core.to_string();
                changed = true;
            }
        }
        if changed {
            out.push(nl);
        }
    }
    out
}

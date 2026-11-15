//! 이미지 픽셀 처리 — OCR 로 글자를 읽어 정보를 찾고, 그 영역만 흐리게(블러) 처리한다.
//!
//! 흐름: 디코딩 → 여러 배율 OCR → 줄 단위 텍스트 검사(공통 Detector) → 단어 좌표로 영역 산출
//!       → 영역 블러 → 원래 형식(PNG/JPEG/BMP)으로 재인코딩

use super::{Ctx, Outcome};
use crate::common::{records, Hit, InfoType, Record};
use crate::ocr::{effective_scale, recognize_scaled, OcrLine};
use crate::rules::{ocr_context, Detector, Lang};
use image::{ImageFormat, RgbaImage};
use std::io::Cursor;

/// 보고서 치환정보 열에 기록되는 표시
pub const BLUR_LABEL: &str = "[흐림처리]";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn union(self, o: Rect) -> Rect {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.w).max(o.x + o.w);
        let y1 = (self.y + self.h).max(o.y + o.h);
        Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
    fn area(&self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }
    /// 교집합 / 작은 쪽 면적 (같은 글자를 다른 배율에서 다시 찾았는지 판단)
    fn overlap_ratio(&self, o: &Rect) -> f32 {
        let w = (self.x + self.w).min(o.x + o.w) - self.x.max(o.x);
        let h = (self.y + self.h).min(o.y + o.h) - self.y.max(o.y);
        if w <= 0.0 || h <= 0.0 {
            return 0.0;
        }
        (w * h) / self.area().min(o.area()).max(1.0)
    }
}

#[derive(Clone, Debug)]
pub struct ImageHit {
    pub rect: Rect,
    pub record: Record,
    /// 점을 넣어 추정한 값이라 검토필요로 표시했음
    pub guessed: bool,
}

/// OCR 줄 → (텍스트, 단어별 바이트 범위)
fn line_text(l: &OcrLine) -> (String, Vec<(usize, usize)>) {
    let mut text = String::new();
    let mut spans = Vec::with_capacity(l.words.len());
    for (i, w) in l.words.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        let s = text.len();
        text.push_str(&w.text);
        spans.push((s, text.len()));
    }
    (text, spans)
}

/// 검출 범위가 걸친 단어들의 영역. 단어 일부만 해당하면 글자 비율로 잘라낸다.
fn hit_rect(text: &str, spans: &[(usize, usize)], l: &OcrLine, h: &Hit) -> Option<Rect> {
    let mut acc: Option<Rect> = None;
    for (w, &(ws, we)) in l.words.iter().zip(spans) {
        let (a, b) = (h.start.max(ws), h.end.min(we));
        if a >= b {
            continue;
        }
        let n = text[ws..we].chars().count().max(1) as f32;
        let ca = text[ws..a].chars().count() as f32;
        let cb = text[ws..b].chars().count() as f32;
        let r = Rect { x: w.x + w.w * ca / n, y: w.y, w: w.w * (cb - ca) / n, h: w.h };
        acc = Some(acc.map_or(r, |p| p.union(r)));
    }
    acc
}

/// 이미지에서 정보 검출 (블러 전 단계)
pub fn detect(img: &RgbaImage, det: &Detector) -> Result<Vec<ImageHit>, String> {
    let mut found: Vec<ImageHit> = Vec::new();
    let o = &det.opts;
    // 1차: 자체 경량 OCR — 원본 1배 한 장, 숫자·영문만 (빠르고 메모리 적음).
    // 확신 없는 글자(?)가 섞인 번호 모양 줄만 모아 한 장으로 붙여 Windows OCR 로 한 번 더 읽는다.
    // 기본·중간 단계(재검사) 모두 이 방식 — Windows OCR 로 그림 전체를 읽는 것은 최고 단계만.
    let lite = o.ocr_lite && crate::ocr::glyph::model().is_some();
    let (mut sure, mut unsure) = (Vec::new(), Vec::new());
    if lite {
        (sure, unsure) = crate::ocr::lite::recognize(img).into_iter().partition(|l| !l.words.iter().any(uncertain));
        if o.depth <= 1 {
            let second = if unsure.is_empty() { Vec::new() } else { recognize_strips(img, &unsure, o).unwrap_or_default() };
            let ip_ctx = [&sure, &unsure, &second].iter().any(|ls| ls.iter().any(|l| l.words.iter().any(|w| ocr_context::has_ipv4(&w.text))));
            // 확신 있는 줄 → 다시 읽은 줄 → 자체 판독의 불확실한 줄 순 (같은 자리는 먼저 찾은 것 1건)
            for ls in [sure, second, unsure] {
                scan_lines_ctx(ls, ip_ctx, det, &mut found);
            }
            return Ok(resolve_guesses(found));
        }
    }
    // Windows OCR (lite 를 끄거나 최고 단계) — 인식할 그림 (좌표는 모두 같음, 흐림은 호출 쪽에서 원본에 적용)
    //  기본: 이진화 1장 / 중간: 이진화 + 원본 / 최고: + 막대 제거 후 이진화
    let mut variants: Vec<RgbaImage> = Vec::new();
    if o.ocr_enhance {
        variants.push(crate::ocr::enhance(img));
    }
    if !o.ocr_enhance || o.depth >= 1 {
        variants.push(img.clone());
    }
    if o.depth >= 2 {
        variants.push(crate::ocr::enhance(&crate::ocr::remove_bars(img)));
    }
    let mut scales = o.ocr_scales.clone();
    if o.depth >= 1 && !scales.iter().any(|s| (s - 1.5).abs() < 0.01) {
        scales.push(1.5);
    }
    // 최고 단계: 자체 학습 경량 OCR 로 숫자 단어를 다시 읽음 (막대 제거한 원본에서) — 맨 먼저 넣어 같은 자리의 다른 판독보다 우선
    if o.depth >= 2 && crate::ocr::glyph::model().is_some() {
        let clean = crate::ocr::remove_bars(img);
        let base = &variants[0];
        let mut done: Vec<f32> = Vec::new();
        for &req in &scales {
            let scale = effective_scale(base, req, o.ocr_max_pixels)?;
            if done.iter().any(|d| (d - scale).abs() < 0.1) {
                continue;
            }
            done.push(scale);
            let lines = recognize_scaled(base, scale)?;
            for line in crate::ocr::glyph::reread(&clean, &lines, 0.9) {
                scan_line(&line, &[], det, &mut found);
            }
        }
    }
    // 최고 단계: (다시 읽기) → 자체 OCR 의 확신 있는 줄 → Windows OCR 전체 → 자체 OCR 의 불확실한 줄
    scan_lines(sure, det, &mut found);
    for img in &variants {
        detect_variant(img, &scales, det, &mut found)?;
    }
    scan_lines(unsure, det, &mut found);
    Ok(resolve_guesses(found))
}

/// 추정값과 같은 값이 그림 어딘가에서 교정 없이 그대로 읽혔으면 확인된 것으로 보고 검토필요를 푼다
fn resolve_guesses(mut found: Vec<ImageHit>) -> Vec<ImageHit> {
    let sure: std::collections::HashSet<String> = found.iter().filter(|f| !f.guessed).map(|f| f.record.original.clone()).collect();
    for f in found.iter_mut().filter(|f| f.guessed && sure.contains(&f.record.original)) {
        f.record.review = false;
        f.guessed = false;
    }
    found
}

/// 그림 1장을 여러 배율로 인식해 `found` 에 더함 (같은 자리는 1건)
fn detect_variant(img: &RgbaImage, scales: &[f32], det: &Detector, found: &mut Vec<ImageHit>) -> Result<(), String> {
    let mut done: Vec<f32> = Vec::new();
    for &req in scales {
        let scale = effective_scale(img, req, det.opts.ocr_max_pixels)?;
        // 상한 때문에 같은 배율이 되면 중복 인식 생략
        if done.iter().any(|d| (d - scale).abs() < 0.1) {
            continue;
        }
        done.push(scale);
        scan_lines(recognize_scaled(img, scale)?, det, found);
    }
    Ok(())
}

/// 인식한 줄들 검사 (두 엔진 공용) — 맥락 교정본을 먼저(교정된 값이 보고서에 남음), 그다음 원문
fn scan_lines(lines: Vec<OcrLine>, det: &Detector, found: &mut Vec<ImageHit>) {
    // 그림 속에 올바른 IP 가 하나라도 있으면, 점이 빠진 IP 모양도 IP 로 추정한다
    let ip_ctx = lines.iter().any(|l| l.words.iter().any(|w| ocr_context::has_ipv4(&w.text)));
    scan_lines_ctx(lines, ip_ctx, det, found)
}

fn scan_lines_ctx(lines: Vec<OcrLine>, ip_ctx: bool, det: &Detector, found: &mut Vec<ImageHit>) {
    for line in lines {
        if let Some((fixed, guessed)) = corrected_line(&line, ip_ctx) {
            scan_line(&fixed, &guessed, det, found);
        }
        scan_line(&line, &[], det, found);
    }
}

/// OCR 오인식 글자를 맥락으로 교정한 줄 (교정할 것이 없으면 None).
/// 두 번째 값: 단어별 "점을 넣어 추정했음" — 그 자리 검출은 검토필요로 표시한다.
fn corrected_line(line: &OcrLine, ip_ctx: bool) -> Option<(OcrLine, Vec<bool>)> {
    let (raw, _) = line_text(line);
    let ctx = ocr_context::line_has_numeric_context(&raw);
    let mut out = line.clone();
    let mut guessed = vec![false; out.words.len()];
    let mut changed = false;
    for (w, g) in out.words.iter_mut().zip(&mut guessed) {
        if let Some(t) = ocr_context::fix_word(&w.text, ctx) {
            w.text = t;
            changed = true;
        }
        if ip_ctx {
            if let Some(t) = ocr_context::repair_dropped_dots(&w.text) {
                w.text = t;
                *g = true;
                changed = true;
            }
        }
    }
    changed.then_some((out, guessed))
}

fn scan_line(line: &OcrLine, guessed: &[bool], det: &Detector, found: &mut Vec<ImageHit>) {
    let (text, spans) = line_text(line);
    for h in det.scan(&text, Lang::None) {
        if h.kind == InfoType::Comment {
            continue;
        }
        let Some(rect) = hit_rect(&text, &spans, line, &h) else { continue };
        // 다른 배율/교정본에서 이미 찾은 같은 위치는 1건으로 본다
        if found.iter().any(|f| f.rect.overlap_ratio(&rect) > 0.3) {
            continue;
        }
        // 점을 넣어 추정한 단어에 걸친 검출은 검토필요
        let guess = spans.iter().zip(guessed).any(|(&(ws, we), &g)| g && h.start < we && ws < h.end);
        if let Some(mut rec) = records(&text, &[h]).pop() {
            rec.masked = BLUR_LABEL.to_string();
            let guessed = guess && !rec.review;
            rec.review |= guess;
            found.push(ImageHit { rect, record: rec, guessed });
        }
    }
}

/// 자체 OCR 이 확신하지 못한 번호 모양 단어: 모르는 글자(?)와 숫자 2개 이상 또는 @·: 가 함께 있음.
/// (앞에 붙은 아이콘이 `0` 으로 읽혀 번호에 붙는 경우가 있어, 단어 어디든 ? 가 있으면 다시 읽는다)
fn uncertain(w: &crate::ocr::OcrWord) -> bool {
    w.text.contains('?') && (w.text.chars().filter(|c| c.is_ascii_digit()).count() >= 2 || w.text.contains(['@', ':']))
}

/// 불확실한 줄들만 잘라 세로로 이어 붙인 한 장을 Windows OCR(이진화, 3배)로 읽고 원본 좌표로 되돌린다.
/// 그림 전체를 여러 배율로 읽는 것보다 훨씬 작은 그림 한 장 — 시간·메모리가 줄 수에 비례.
fn recognize_strips(img: &RgbaImage, lines: &[OcrLine], o: &crate::config::Options) -> Result<Vec<OcrLine>, String> {
    const GAP: u32 = 12;
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    // 줄 상자 + 여백 (세로 0.3줄, 가로 0.5줄)
    let mut boxes: Vec<(u32, u32, u32, u32)> = lines
        .iter()
        .filter_map(|l| {
            let x0 = l.words.iter().map(|w| w.x).fold(f32::MAX, f32::min);
            let y0 = l.words.iter().map(|w| w.y).fold(f32::MAX, f32::min);
            let x1 = l.words.iter().map(|w| w.x + w.w).fold(f32::MIN, f32::max);
            let y1 = l.words.iter().map(|w| w.y + w.h).fold(f32::MIN, f32::max);
            let lh = (y1 - y0).max(1.0);
            let (a, b) = ((x0 - lh * 0.5).max(0.0), (y0 - lh * 0.3).max(0.0));
            let (c, d) = ((x1 + lh * 0.5).min(iw), (y1 + lh * 0.3).min(ih));
            (c - a >= 2.0 && d - b >= 2.0).then(|| (a as u32, b as u32, (c - a) as u32, (d - b) as u32))
        })
        .collect();
    boxes.sort_by_key(|b| (b.1, b.0));
    let width = boxes.iter().map(|b| b.2).max().unwrap_or(0);
    let height: u32 = boxes.iter().map(|b| b.3 + GAP).sum::<u32>() + GAP;
    if width == 0 {
        return Ok(Vec::new());
    }
    let mut sheet = RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
    let mut strips = Vec::with_capacity(boxes.len()); // (시트 y, 원본 상자)
    let mut y = GAP;
    for b in &boxes {
        let crop = image::imageops::crop_imm(img, b.0, b.1, b.2, b.3).to_image();
        // 밝은 글자 줄은 반전해 붙임 (시트 전체 이진화의 극성을 맞춤)
        let mut crop = crop;
        let mean: u64 = crop.pixels().map(|p| (p[0] as u64 * 299 + p[1] as u64 * 587 + p[2] as u64 * 114) / 1000).sum::<u64>() / (crop.width() as u64 * crop.height() as u64).max(1);
        if mean < 128 {
            image::imageops::invert(&mut crop);
        }
        image::imageops::replace(&mut sheet, &crop, 0, y as i64);
        strips.push((y, *b));
        y += b.3 + GAP;
    }
    let sheet = if o.ocr_enhance { crate::ocr::enhance(&sheet) } else { sheet };
    let scale = effective_scale(&sheet, 3.0, o.ocr_max_pixels)?;
    let mut out = Vec::new();
    for mut l in recognize_scaled(&sheet, scale)? {
        let cy = l.words.iter().map(|w| w.y + w.h / 2.0).sum::<f32>() / l.words.len().max(1) as f32;
        let Some(&(sy, b)) = strips.iter().find(|(sy, b)| cy >= *sy as f32 && cy < (*sy + b.3) as f32) else { continue };
        for w in &mut l.words {
            w.x += b.0 as f32;
            w.y += b.1 as f32 - sy as f32;
        }
        out.push(l);
    }
    Ok(out)
}

/// 모자이크 후 가우시안 블러 — 확대·선명화로 복원되지 않도록 두 단계로 흐린다.
fn obscure(sub: &RgbaImage, block: u32, sigma: f32) -> RgbaImage {
    let mut out = mosaic(sub, block);
    gauss_approx(&mut out, sigma);
    out
}

/// 모자이크: `block`×`block` 칸마다 평균색으로 채움 (끝의 덜 찬 칸은 그 칸의 평균)
fn mosaic(img: &RgbaImage, block: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let b = block.max(1);
    let src: &[u8] = img;
    let mut out = vec![0u8; src.len()];
    let mut acc = vec![0u32; (w.div_ceil(b) * 4) as usize];
    for by in (0..h).step_by(b as usize) {
        let y1 = (by + b).min(h);
        acc.iter_mut().for_each(|v| *v = 0);
        for y in by..y1 {
            let row = &src[(y * w * 4) as usize..((y + 1) * w * 4) as usize];
            for (x, px) in row.chunks_exact(4).enumerate() {
                let k = (x as u32 / b * 4) as usize;
                for c in 0..4 {
                    acc[k + c] += px[c] as u32;
                }
            }
        }
        for bx in 0..w.div_ceil(b) {
            let x0 = bx * b;
            let x1 = (x0 + b).min(w);
            let n = (x1 - x0) * (y1 - by);
            let k = (bx * 4) as usize;
            let avg: [u8; 4] = std::array::from_fn(|c| ((acc[k + c] + n / 2) / n) as u8);
            for y in by..y1 {
                for x in x0..x1 {
                    let i = ((y * w + x) * 4) as usize;
                    out[i..i + 4].copy_from_slice(&avg);
                }
            }
        }
    }
    RgbaImage::from_raw(w, h, out).expect("크기 같음")
}

/// 가우시안 근사 — 상자 흐림 3번 (중심극한정리로 가우시안에 수렴).
/// 누적합으로 픽셀당 계산량이 반경과 무관하다 (진짜 가우시안은 σ=17 이면 픽셀당 100번 곱셈).
/// 두 방향 모두 메모리를 줄 순서대로 읽는다 (세로도 열마다가 아니라 줄 단위 누적합).
fn gauss_approx(img: &mut RgbaImage, sigma: f32) {
    // 상자 3개의 반경: 분산 합이 σ² 가 되도록 (W. Jarosz, "Fast Image Convolutions")
    let n = 3.0f32;
    let wi = ((12.0 * sigma * sigma / n) + 1.0).sqrt().floor() as i32;
    let wl = if wi % 2 == 0 { wi - 1 } else { wi };
    let wu = wl + 2;
    let m = ((12.0 * sigma * sigma - n * (wl * wl) as f32 - 4.0 * n * wl as f32 - 3.0 * n) / (-4.0 * wl as f32 - 4.0)).round() as i32;
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return;
    }
    let mut tmp = vec![0u8; w * h * 4];
    for i in 0..3 {
        let r = (((if i < m { wl } else { wu }) - 1) / 2).max(0) as usize;
        if r == 0 {
            continue;
        }
        let buf: &mut [u8] = img;
        box_h(buf, &mut tmp, w, r);
        box_v(&tmp, buf, w, h, r);
    }
}

/// 나눗셈 대신 곱셈: 1/(2r+1) 을 2^24 고정소수점으로 (픽셀마다 나누면 나눗셈이 대부분의 시간을 먹는다)
fn recip(r: usize) -> u64 {
    (1u64 << 24) / (2 * r as u64 + 1)
}

#[inline]
fn scale(acc: u32, inv: u64) -> u8 {
    ((acc as u64 * inv + (1 << 23)) >> 24).min(255) as u8
}

/// 가로 상자 흐림 (줄마다 RGBA 4채널 누적합). 가장자리는 끝 값 반복.
fn box_h(src: &[u8], dst: &mut [u8], w: usize, r: usize) {
    let inv = recip(r);
    for (srow, drow) in src.chunks_exact(w * 4).zip(dst.chunks_exact_mut(w * 4)) {
        let at = |x: usize, c: usize| srow[x.min(w - 1) * 4 + c] as u32;
        let mut acc: [u32; 4] = std::array::from_fn(|c| at(0, c) * (r as u32 + 1) + (1..=r).map(|x| at(x, c)).sum::<u32>());
        for x in 0..w {
            for c in 0..4 {
                drow[x * 4 + c] = scale(acc[c], inv);
                acc[c] = acc[c] + at(x + r + 1, c) - at(x.saturating_sub(r), c);
            }
        }
    }
}

/// 세로 상자 흐림 — 열 누적합 배열 하나를 두고 줄 단위로 내려간다.
fn box_v(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    let inv = recip(r);
    let rw = w * 4;
    let row = |y: usize| &src[y.min(h - 1) * rw..(y.min(h - 1) + 1) * rw];
    let mut acc: Vec<u32> = row(0).iter().map(|&v| v as u32 * (r as u32 + 1)).collect();
    for y in 1..=r {
        for (a, &v) in acc.iter_mut().zip(row(y)) {
            *a += v as u32;
        }
    }
    for y in 0..h {
        let d = &mut dst[y * rw..(y + 1) * rw];
        for (o, &a) in d.iter_mut().zip(&acc) {
            *o = scale(a, inv);
        }
        let (add, sub) = (row(y + r + 1), row(y.saturating_sub(r)));
        for ((a, &p), &q) in acc.iter_mut().zip(add).zip(sub) {
            *a = *a + p as u32 - q as u32;
        }
    }
}

/// 지정 영역만 흐리게
pub fn blur_rect(img: &mut RgbaImage, r: Rect, strength: f32) {
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    // 세로 여유 + 가로로 반 글자 폭 여유 (OCR 이 끝 글자를 놓쳐도 노출되지 않도록)
    let pad = (r.h * 0.25).max(2.0);
    let hpad = (r.h * 0.5).max(3.0);
    let x0 = (r.x - hpad).floor().clamp(0.0, iw) as u32;
    let y0 = (r.y - pad).floor().clamp(0.0, ih) as u32;
    let x1 = (r.x + r.w + hpad).ceil().clamp(0.0, iw) as u32;
    let y1 = (r.y + r.h + pad).ceil().clamp(0.0, ih) as u32;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let sigma = (r.h * 0.6).max(4.0) * strength.max(0.1);
    let block = ((r.h / 2.0) as u32).max(3);
    let sub = image::imageops::crop_imm(img, x0, y0, x1 - x0, y1 - y0).to_image();
    let b = obscure(&sub, block, sigma);
    image::imageops::replace(img, &b, x0 as i64, y0 as i64);
}

/// 이미지 전체를 흐리게 (문서 미리보기 썸네일용)
pub fn blur_all(img: &mut RgbaImage, strength: f32) {
    let longest = img.width().max(img.height()) as f32;
    let sigma = (longest / 60.0).max(2.0) * strength.max(0.1);
    *img = obscure(img, ((longest / 80.0) as u32).max(2), sigma);
}

pub fn encode(img: &RgbaImage, fmt: ImageFormat, keep_alpha: bool) -> Result<Vec<u8>, String> {
    let mut out = Cursor::new(Vec::new());
    let dynimg = image::DynamicImage::ImageRgba8(img.clone());
    let res = match fmt {
        ImageFormat::Jpeg => {
            let rgb = dynimg.to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92).encode_image(&rgb)
        }
        _ if keep_alpha => dynimg.write_to(&mut out, fmt),
        _ => image::DynamicImage::ImageRgb8(dynimg.to_rgb8()).write_to(&mut out, fmt),
    };
    res.map_err(|e| format!("이미지 저장 실패: {e}"))?;
    Ok(out.into_inner())
}

/// 이미지 파일 처리: OCR → 정보 영역 흐림 → 재인코딩
pub fn process(bytes: &[u8], fmt: ImageFormat, ctx: &Ctx) -> Outcome {
    let opts = &ctx.det.opts;
    if !opts.ocr {
        return Outcome::default();
    }
    let dynimg = match image::load_from_memory_with_format(bytes, fmt) {
        Ok(i) => i,
        Err(e) => return Outcome { notes: vec![format!("이미지 디코딩 실패(OCR 생략): {e}")], ..Default::default() },
    };
    let keep_alpha = dynimg.color().has_alpha();
    let mut rgba = dynimg.to_rgba8();
    let records = match process_rgba(&mut rgba, ctx) {
        Ok(r) => r,
        Err(e) => return Outcome { notes: vec![e], ..Default::default() },
    };
    if records.is_empty() {
        return Outcome::default();
    }
    match encode(&rgba, fmt, keep_alpha) {
        Ok(d) => Outcome { data: Some(d), records, ..Default::default() },
        Err(e) => Outcome { notes: vec![e], ..Default::default() },
    }
}

/// 픽셀 버퍼 직접 처리 (OCR → 흐림). PDF 내장 이미지 등에서 공용으로 사용.
pub fn process_rgba(img: &mut RgbaImage, ctx: &Ctx) -> Result<Vec<Record>, String> {
    let hits = detect(img, ctx.det)?;
    for h in &hits {
        blur_rect(img, h.rect, ctx.det.opts.blur_strength);
    }
    Ok(hits.into_iter().map(|h| h.record).collect())
}

/// 바이트열 전체를 흐리게 (형식 자동 판별, 미지원 형식이면 None)
pub fn blur_whole_bytes(bytes: &[u8], strength: f32) -> Option<Vec<u8>> {
    let fmt = image::guess_format(bytes).ok()?;
    if !matches!(fmt, ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Bmp) {
        return None;
    }
    let dynimg = image::load_from_memory_with_format(bytes, fmt).ok()?;
    let keep_alpha = dynimg.color().has_alpha();
    let mut rgba = dynimg.to_rgba8();
    blur_all(&mut rgba, strength);
    encode(&rgba, fmt, keep_alpha).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 상자 3번 근사가 진짜 가우시안과 거의 같은 결과인가 (평균 차이 2단계 미만, 최대 차이 작음)
    #[test]
    fn box_blur_matches_gaussian() {
        let mut s = 12345u32;
        let img = RgbaImage::from_fn(260, 220, |x, y| {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            let v = if (x / 7 + y / 5) % 2 == 0 { 30 } else { 220 };
            image::Rgba([v, (s >> 16) as u8, (x * 3) as u8, 255])
        });
        for sigma in [2.0f32, 4.0, 9.0, 17.0] {
            let want = image::imageops::blur(&img, sigma);
            let mut got = img.clone();
            gauss_approx(&mut got, sigma);
            // 가장자리 처리 방식이 달라 안쪽만 비교
            let m = (3.0 * sigma) as u32;
            let (mut sum, mut n, mut max) = (0u64, 0u64, 0i32);
            for y in m..220 - m {
                for x in m..260 - m {
                    for c in 0..3 {
                        let d = (want.get_pixel(x, y)[c] as i32 - got.get_pixel(x, y)[c] as i32).abs();
                        sum += d as u64;
                        n += 1;
                        max = max.max(d);
                    }
                }
            }
            assert!(n > 10_000);
            let mean = sum as f64 / n as f64;
            assert!(mean < 2.0 && max < 20, "sigma {sigma}: 평균 {mean:.2}, 최대 {max}");
        }
    }

    /// 모자이크: 칸 안은 모두 같은 색 = 칸 평균 (끝의 덜 찬 칸 포함)
    #[test]
    fn mosaic_fills_block_average() {
        let img = RgbaImage::from_fn(10, 7, |x, y| image::Rgba([(x * 10) as u8, (y * 20) as u8, 0, 255]));
        let m = mosaic(&img, 4);
        // 첫 칸 x 0..4, y 0..4 → R 평균 15, G 평균 30
        assert_eq!(m.get_pixel(0, 0).0, [15, 30, 0, 255]);
        assert_eq!(m.get_pixel(3, 3).0, [15, 30, 0, 255]);
        // 끝 칸 x 8..10, y 4..7 → R 85, G 100
        assert_eq!(m.get_pixel(9, 6).0, [85, 100, 0, 255]);
    }
}

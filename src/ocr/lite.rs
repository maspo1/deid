//! 자체 경량 OCR 엔진 (기본) — 개인정보 검사에 필요한 글자(숫자·영문·기호)만 읽는다.
//!
//! Windows OCR 은 한글 문장 전체를 읽느라 그림마다 여러 배율로 확대해 수 초씩 걸리고 메모리를 많이 쓴다.
//! 이 엔진은 원본 크기(1배)의 회색조 한 장으로 글자 줄을 찾고, 줄마다 작은 조각만 확대해 글자 단위 신경망으로 읽는다.
//!
//! 1. 회색조 + 오츠 임계값 → 어두운 글자/밝은 글자 두 극성
//! 2. 연결 성분(가로 연속 구간 + 합집합 찾기 — 화소별 라벨 배열 없이 구간 수만큼만 메모리)
//! 3. 극성 확인: 성분 둘레 배경이 반대 밝기여야 글자 (검은 글자 속 구멍이 밝은 글자로 잡히지 않게)
//! 4. 글자 크기 성분을 높이가 겹치는 것끼리 줄로 묶고, 점·하이픈 같은 작은 성분은 그 줄에 붙임
//! 5. 줄 조각 → `glyph::segment_gray` → 글자마다 MLP 분류 → 글자 사이 간격으로 단어를 나눔
//!
//! 결과는 Windows OCR 과 같은 `OcrLine`(원본 좌표) — 이후 검사·흐림은 같은 코드를 쓴다.
//! 한글은 "그 밖"(?)으로 나온다 (한글 맥락 키워드가 필요한 검사는 Windows OCR 단계가 담당).

use super::glyph::{self, CLASSES};
use super::{OcrLine, OcrWord};
use image::{GrayImage, Luma, RgbaImage};

/// 글자로 볼 성분 높이 (원본 화소)
const MIN_H: u32 = 5;
const MAX_H: u32 = 160;
/// 이 확률 미만인 글자는 '?' (모르는 글자)
const MIN_P: f32 = 0.5;

#[derive(Clone, Copy, Debug)]
struct Comp {
    x0: u32,
    y0: u32,
    x1: u32, // 포함
    y1: u32, // 포함
    area: u32,
}

impl Comp {
    fn h(&self) -> u32 {
        self.y1 - self.y0 + 1
    }
    fn w(&self) -> u32 {
        self.x1 - self.x0 + 1
    }
}

fn gray_of(img: &RgbaImage) -> GrayImage {
    GrayImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        let a = p[3] as u32;
        let l = (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114) / 1000;
        // 투명 → 흰 바탕
        Luma([((l * a + 255 * (255 - a)) / 255) as u8])
    })
}

fn otsu(g: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in g.pixels() {
        hist[p[0] as usize] += 1;
    }
    let total = g.pixels().len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let (mut wb, mut sb, mut best, mut th) = (0f64, 0f64, -1f64, 127u8);
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
            th = t as u8;
        }
    }
    th
}

fn find(p: &mut [u32], mut x: u32) -> u32 {
    while p[x as usize] != x {
        p[x as usize] = p[p[x as usize] as usize];
        x = p[x as usize];
    }
    x
}

/// 연결 성분 (8방향) — 줄마다 잉크 구간을 만들고 윗줄 구간과 겹치면 합친다.
fn components(g: &GrayImage, ink: impl Fn(u8) -> bool) -> Vec<Comp> {
    let (w, h) = g.dimensions();
    let raw = g.as_raw();
    let mut runs: Vec<(u32, u32, u32)> = Vec::new(); // (y, x0, x1 포함)
    let mut parent: Vec<u32> = Vec::new();
    let mut prev: (usize, usize) = (0, 0); // 윗줄 구간 범위
    for y in 0..h {
        let row = &raw[(y * w) as usize..((y + 1) * w) as usize];
        let start = runs.len();
        let mut x = 0;
        while x < w {
            if !ink(row[x as usize]) {
                x += 1;
                continue;
            }
            let s = x;
            while x < w && ink(row[x as usize]) {
                x += 1;
            }
            let id = runs.len() as u32;
            runs.push((y, s, x - 1));
            parent.push(id);
            // 윗줄에서 8방향으로 닿는 구간과 합침 (윗줄 구간도 x 순 — 지나간 구간은 다시 보지 않음)
            while prev.0 < prev.1 && runs[prev.0].2 + 1 < s {
                prev.0 += 1;
            }
            let mut j = prev.0;
            while j < prev.1 && runs[j].1 <= x {
                let (r1, r2) = (find(&mut parent, j as u32), find(&mut parent, id));
                if r1 != r2 {
                    parent[r1.max(r2) as usize] = r1.min(r2);
                }
                j += 1;
            }
        }
        prev = (start, runs.len());
    }
    // 뿌리별 경계 상자
    let mut idx = vec![u32::MAX; runs.len()];
    let mut out: Vec<Comp> = Vec::new();
    for i in 0..runs.len() {
        let r = find(&mut parent, i as u32) as usize;
        let (y, a, b) = runs[i];
        if idx[r] == u32::MAX {
            idx[r] = out.len() as u32;
            out.push(Comp { x0: a, y0: y, x1: b, y1: y, area: 0 });
        }
        let c = &mut out[idx[r] as usize];
        c.x0 = c.x0.min(a);
        c.x1 = c.x1.max(b);
        c.y0 = c.y0.min(y);
        c.y1 = c.y1.max(y);
        c.area += b - a + 1;
    }
    out
}

/// 성분 둘레 고리의 평균 밝기 — 상자에서 높이의 절반만큼 떨어진 테두리를 따라 표본.
/// 진짜 글자면 고리는 배경(어두운 글자 → 밝음, 밝은 글자 → 어두움)이고,
/// 검은 글자 속 구멍(밝은 성분)은 고리가 글자 바깥 흰 바탕에 놓여 밝으므로 밝은 글자로 잡히지 않는다.
fn ring_mean(g: &GrayImage, c: &Comp) -> u32 {
    let d = (c.h() / 2).max(2) as i64;
    let (w, h) = (g.width() as i64, g.height() as i64);
    let (x0, y0, x1, y1) = (c.x0 as i64 - d, c.y0 as i64 - d, c.x1 as i64 + d, c.y1 as i64 + d);
    let (mut s, mut n) = (0u32, 0u32);
    let mut at = |x: i64, y: i64| {
        if x >= 0 && y >= 0 && x < w && y < h {
            s += g.get_pixel(x as u32, y as u32)[0] as u32;
            n += 1;
        }
    };
    let step = ((x1 - x0) / 16).max(1) as usize;
    for x in (x0..=x1).step_by(step) {
        at(x, y0);
        at(x, y1);
    }
    let step = ((y1 - y0) / 8).max(1) as usize;
    for y in (y0..=y1).step_by(step) {
        at(x0, y);
        at(x1, y);
    }
    if n == 0 {
        return 128;
    }
    s / n
}

/// 한 줄 = 글자 크기 성분들 + 붙은 작은 성분들
struct Line {
    comps: Vec<Comp>,
    y0: u32,
    y1: u32,
    x1: u32,
}

/// 성분 → 줄. 글자 크기 성분은 높이 겹침으로, 작은 성분(점·하이픈·쉼표)은 들어갈 줄을 찾아 붙인다.
fn lines_of(comps: &[Comp]) -> Vec<Line> {
    let mut tall: Vec<Comp> = comps.iter().copied().filter(|c| c.h() >= MIN_H && c.h() <= MAX_H && c.w() <= 3 * c.h() + 4).collect();
    tall.sort_by_key(|c| (c.x0, c.y0));
    let mut lines: Vec<Line> = Vec::new();
    for c in tall {
        // 높이가 충분히 겹치고, 가로로 너무 멀지 않은 줄 중 가장 잘 겹치는 것
        let best = lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                let ov = (l.y1.min(c.y1) as i64 - l.y0.max(c.y0) as i64 + 1).max(0) as u32;
                let lh = l.y1 - l.y0 + 1;
                let near = c.x0 <= l.x1 + 2 * lh.max(c.h());
                let similar = c.h() * 3 >= lh && lh * 3 >= c.h() * 2;
                (near && similar && ov * 10 >= 6 * c.h().min(lh)).then_some((i, ov))
            })
            .max_by_key(|&(_, ov)| ov)
            .map(|(i, _)| i);
        match best {
            Some(i) => {
                let l = &mut lines[i];
                l.comps.push(c);
                l.x1 = l.x1.max(c.x1);
                // 줄 높이는 처음 글자 기준을 유지하되 큰 글자에 맞춰 조금씩 넓힘
                l.y0 = l.y0.min(c.y0);
                l.y1 = l.y1.max(c.y1);
            }
            None => lines.push(Line { comps: vec![c], y0: c.y0, y1: c.y1, x1: c.x1 }),
        }
    }
    // 작은 성분: 중심이 줄 높이 안에 있고 가로로 줄 범위 근처인 줄에 붙임
    for c in comps.iter().filter(|c| c.h() < MIN_H && c.w() <= 2 * MIN_H + 4 && c.area >= 2) {
        let cy = (c.y0 + c.y1) / 2;
        let lx0 = |l: &Line| l.comps.iter().map(|k| k.x0).min().unwrap_or(0);
        if let Some(l) = lines.iter_mut().find(|l| {
            let lh = l.y1 - l.y0 + 1;
            cy >= l.y0 && cy <= l.y1 && c.x0 + lh >= lx0(l) && c.x0 <= l.x1 + lh
        }) {
            l.comps.push(*c);
        }
    }
    lines
}

/// 그림 → 줄·단어 (원본 좌표). 모델이 없으면 빈 결과.
pub fn recognize(img: &RgbaImage) -> Vec<OcrLine> {
    let Some(model) = glyph::model() else { return vec![] };
    let g = gray_of(img);
    if g.width() < 4 || g.height() < MIN_H {
        return vec![];
    }
    let th = otsu(&g);
    let mut out = Vec::new();
    for light in [false, true] {
        let comps: Vec<Comp> = if light { components(&g, |v| v > th) } else { components(&g, |v| v <= th) }
            .into_iter()
            // 극성 확인: 둘레가 반대 밝기 (어두운 글자 → 밝은 배경)
            .filter(|c| c.h() <= MAX_H && c.w() <= 8 * MAX_H)
            .filter(|c| {
                let bg = ring_mean(&g, c);
                if light {
                    bg < th as u32
                } else {
                    bg > th as u32
                }
            })
            .collect();
        for line in lines_of(&comps) {
            read_line(&g, &line, light, model, &mut out);
        }
    }
    out
}

/// 줄 하나: 큰 간격(줄 높이 1.5배 초과)에서 구역을 나눠 조각마다 글자를 읽고, 글자 사이 간격으로 단어를 나눈다.
/// (1배 크기에서는 작은 점이 잉크로 잡히지 않을 수 있어 단어 간격을 여기서 정하지 않는다 — 확대한 조각에서 정함)
fn read_line(g: &GrayImage, line: &Line, light: bool, model: &glyph::Mlp, out: &mut Vec<OcrLine>) {
    let mut cs = line.comps.clone();
    cs.sort_by_key(|c| c.x0);
    let lh = line.y1 - line.y0 + 1;
    let mut start = 0;
    let mut reach = cs[0].x1;
    for i in 1..=cs.len() {
        if i < cs.len() && cs[i].x0 <= reach + (lh * 3) / 2 {
            reach = reach.max(cs[i].x1);
            continue;
        }
        let part = &cs[start..i];
        if part.iter().any(|c| c.h() >= MIN_H) {
            if let Some(l) = read_part(g, part, light, model) {
                out.push(l);
            }
        }
        if i < cs.len() {
            start = i;
            reach = cs[i].x1;
        }
    }
}

fn read_part(g: &GrayImage, part: &[Comp], light: bool, model: &glyph::Mlp) -> Option<OcrLine> {
    let x0 = part.iter().map(|c| c.x0).min()?.saturating_sub(1);
    let x1 = (part.iter().map(|c| c.x1).max()? + 2).min(g.width());
    let y0 = part.iter().map(|c| c.y0).min()?.saturating_sub(1);
    let y1 = (part.iter().map(|c| c.y1).max()? + 2).min(g.height());
    if x1 <= x0 + 1 || y1 <= y0 + 3 {
        return None;
    }
    let crop = image::imageops::crop_imm(g, x0, y0, x1 - x0, y1 - y0).to_image();
    // 붙은 글자 나누기 판단: 그 밖(?)이 아닌 글자로 0.9 이상 확신 (애매하면 더 나눠 봄: `-2` → `-` `2`)
    let accept = |f: &[f32; glyph::FEAT]| {
        let p = model.predict(f);
        let (c, &pc) = p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
        CLASSES[c] != '?' && pc >= 0.9
    };
    let (glyphs, k) = glyph::segment_gray(&crop, Some(light), Some(&accept));
    if glyphs.is_empty() {
        return None;
    }
    // 단어 경계: 글자 사이 간격이 글자 높이(큰 글자들의 중앙값)의 0.4배 초과
    let b = glyph::BW * glyph::BH;
    let mut tall: Vec<f32> = glyphs.iter().map(|g| g.feat[b + 3] * BAND).filter(|&gh| gh >= BAND * 0.4).collect();
    tall.sort_by(f32::total_cmp);
    let gh = tall.get(tall.len() / 2).copied().unwrap_or(BAND * 0.7);
    let gap = (gh * 0.4) as u32;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, gl) in glyphs.iter().enumerate() {
        match groups.last_mut() {
            Some(gr) if gl.x0 <= gr.iter().map(|&j| glyphs[j].x1).max().unwrap_or(0) + gap => gr.push(i),
            _ => groups.push(vec![i]),
        }
    }
    let pad = k; // 원본 1px 여백 (학습 때 단어를 잉크에 맞춰 0~2px 여백으로 자른 것과 같게)
    let mut words: Vec<(String, u32, u32)> = Vec::new();
    for gr in &groups {
        // 위치·크기 특징을 줄 높이가 아니라 이 단어의 잉크 높이 기준으로 다시 계산
        let hh = BAND;
        let wy0 = gr.iter().map(|&j| glyphs[j].feat[b] * hh).fold(f32::MAX, f32::min) - pad;
        let wy1 = gr.iter().map(|&j| glyphs[j].feat[b + 1] * hh).fold(f32::MIN, f32::max) + pad;
        let wh = (wy1 - wy0).max(1.0);
        let mut chars = Vec::with_capacity(gr.len());
        let mut probs: Vec<[f32; CLASSES.len()]> = Vec::with_capacity(gr.len());
        for &j in gr {
            let gl = &glyphs[j];
            let mut f = gl.feat;
            f[b] = (gl.feat[b] * hh - wy0) / wh;
            f[b + 1] = (gl.feat[b + 1] * hh - wy0) / wh;
            f[b + 2] = gl.feat[b + 2] * hh / wh;
            f[b + 3] = gl.feat[b + 3] * hh / wh;
            let p = model.predict(&f);
            let (c, &pc) = p.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
            // 닮은꼴 묶음(0/o, 1/l/i …)은 묶음 확률 합으로 판단 — 둘로 갈려 둘 다 0.5 미만이 되는 것을 막음
            let group = look_alike(CLASSES[c]).map_or(pc, |grp| grp.chars().map(|k| p[glyph::class_of(k)]).sum());
            let ch = if group >= MIN_P { CLASSES[c] } else { '?' };
            chars.push(ch);
            probs.push(p);
        }
        let text = settle_look_alikes(&chars, &probs.iter().collect::<Vec<_>>());
        words.push((text, glyphs[gr[0]].x0, gr.iter().map(|&j| glyphs[j].x1).max().unwrap_or(0)));
    }
    let (fy, fh) = (y0 as f32, (y1 - y0) as f32);
    let words: Vec<OcrWord> = words
        .into_iter()
        // '?' 만으로 된 단어(한글·아이콘)는 검사에 쓸모없음
        .filter(|w| w.0.chars().any(|c| c != '?'))
        .map(|(text, a, b)| OcrWord { text, x: x0 as f32 + a as f32 / k, y: fy, w: (b + 1 - a) as f32 / k, h: fh })
        .collect();
    (!words.is_empty()).then_some(OcrLine { words })
}

/// 확대한 줄 띠 높이 (`glyph::segment_gray` 의 NORM_H)
const BAND: f32 = 48.0;

/// 숫자와 영문 소문자가 서로 닮은 묶음 (b 는 16진수 글자라 넣지 않음)
fn look_alike(c: char) -> Option<&'static str> {
    const GROUPS: [&str; 5] = ["0o", "1li", "5s", "2z", "9gq"];
    GROUPS.iter().copied().find(|g| g.contains(c))
}

/// 단어 안의 닮은꼴(0/o, 1/l/i, 5/s, 2/z, 9/g/q)을 숫자·영문 중 하나로 정한다 (원래 판독 기준이라 연쇄로 번지지 않음).
/// * MAC 모양(두 글자씩 `:`/`-`) → 숫자 (16진수에 o·l·s·z·g 없음)
/// * 이메일 `@` 뒤 도메인 → 영문
/// * 그 밖: 가장 가까운 닮은꼴 아닌 영숫자 이웃(양쪽)이 숫자뿐이면 숫자, 영문뿐이면 영문
fn settle_look_alikes(chars: &[char], probs: &[&[f32; CLASSES.len()]]) -> String {
    let n = chars.len();
    let mac = n == 17 && [2, 5, 8, 11, 14].iter().all(|&i| chars[i] == ':' || chars[i] == '-');
    let at = chars.iter().position(|&c| c == '@');
    let plain = |c: char| c.is_ascii_alphanumeric() && look_alike(c).is_none();
    let mut out: Vec<char> = chars.to_vec();
    for i in 0..n {
        let Some(gr) = look_alike(chars[i]) else { continue };
        let want_digit = if mac {
            true
        } else if at.map_or(false, |a| i > a) {
            false
        } else {
            let left = chars[..i].iter().rev().find(|&&c| plain(c));
            let right = chars[i + 1..].iter().find(|&&c| plain(c));
            let (d, a) = (
                left.map_or(false, |c| c.is_ascii_digit()) || right.map_or(false, |c| c.is_ascii_digit()),
                left.map_or(false, |c| c.is_ascii_alphabetic()) || right.map_or(false, |c| c.is_ascii_alphabetic()),
            );
            match (d, a) {
                (true, false) => true,
                (false, true) => false,
                _ => continue,
            }
        };
        let p = probs[i];
        if let Some(k) = gr.chars().filter(|k| k.is_ascii_digit() == want_digit).max_by(|a, b| p[glyph::class_of(*a)].total_cmp(&p[glyph::class_of(*b)])) {
            out[i] = k;
        }
    }
    out.into_iter().collect()
}

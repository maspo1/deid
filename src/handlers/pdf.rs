//! PDF 처리.
//!
//! 1. 페이지 본문: 콘텐츠 스트림 압축 해제 → 텍스트 연산자(Tj/TJ/'/")를 폰트 인코딩(ToUnicode CMap,
//!    WinAnsi)으로 해석해 글자 단위 위치를 기억 → 공통 Detector 로 검사 → 해당 글자 코드를 `*` 글리프 코드로 교체
//!    (한 줄이 여러 조각으로 출력되는 경우를 위해 "붙여서/띄워서" 두 가지로 이어 검사)
//! 2. 내장 이미지(스캔 문서): JPEG(DCT) / 무손실 RGB·Gray → OCR → 정보 영역 흐림
//! 3. 문자열 객체: 문서 정보(작성자 등), 주석·링크, 양식 값
//! 4. 텍스트 스트림(XMP 메타데이터 등), 첨부파일(재귀 처리)
//!
//! 저장 시 객체 테이블 전체를 새로 쓰므로 증분 저장으로 남아 있던 **이전 판의 원문도 제거**된다.

use super::{process as process_entry, raster, Ctx, Outcome};
use crate::common::{char_mask, decode_text, records, Record};
use crate::rules::Lang;
use image::{ImageFormat, RgbaImage};
use lopdf::content::Content;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{BTreeMap, HashMap, HashSet};

// ───────────────────────────── 폰트 인코딩 ─────────────────────────────

/// 폰트 1개의 코드 ↔ 유니코드 변환기
#[derive(Clone, Debug, Default)]
struct FontCodec {
    two_byte: bool,
    to_uni: HashMap<u32, String>,
    /// 정보 글자를 덮어쓸 코드 (`*` 글리프 등)
    mask_code: u32,
}

/// CMap 토큰
enum Tok {
    Hex(Vec<u8>),
    Open,
    Close,
    Word(String),
}

fn cmap_tokens(s: &str) -> Vec<Tok> {
    let b = s.as_bytes();
    let mut v = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'<' if b.get(i + 1) != Some(&b'<') => {
                let e = s[i + 1..].find('>').map_or(b.len(), |p| i + 1 + p);
                let hex: String = s[i + 1..e].chars().filter(|c| c.is_ascii_hexdigit()).collect();
                let bytes = (0..hex.len() / 2).filter_map(|k| u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).ok()).collect();
                v.push(Tok::Hex(bytes));
                i = e + 1;
            }
            b'[' => {
                v.push(Tok::Open);
                i += 1
            }
            b']' => {
                v.push(Tok::Close);
                i += 1
            }
            c if c.is_ascii_alphabetic() => {
                let st = i;
                while i < b.len() && b[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                v.push(Tok::Word(s[st..i].to_string()));
            }
            _ => i += 1,
        }
    }
    v
}

fn be_code(b: &[u8]) -> u32 {
    b.iter().fold(0u32, |a, &x| (a << 8) | x as u32)
}

fn utf16be(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
    String::from_utf16_lossy(&units)
}

/// ToUnicode CMap 해석 → (코드→문자, 2바이트 코드 여부)
fn parse_cmap(data: &[u8]) -> (HashMap<u32, String>, Option<bool>) {
    let text = String::from_utf8_lossy(data);
    let toks = cmap_tokens(&text);
    let mut map = HashMap::new();
    let mut two = None;
    let mut i = 0;
    let mut mode = "";
    while i < toks.len() {
        match &toks[i] {
            Tok::Word(w) => {
                mode = match w.as_str() {
                    "begincodespacerange" => "space",
                    "beginbfchar" => "char",
                    "beginbfrange" => "range",
                    w if w.starts_with("end") => "",
                    _ => mode,
                };
                i += 1;
            }
            Tok::Hex(lo) if mode == "space" => {
                two.get_or_insert(lo.len() >= 2);
                i += 2;
            }
            Tok::Hex(src) if mode == "char" => {
                if let Some(Tok::Hex(dst)) = toks.get(i + 1) {
                    map.insert(be_code(src), utf16be(dst));
                }
                i += 2;
            }
            Tok::Hex(lo) if mode == "range" => {
                let (Some(Tok::Hex(hi)), Some(next)) = (toks.get(i + 1), toks.get(i + 2)) else { break };
                let (lo, hi) = (be_code(lo), be_code(hi));
                match next {
                    Tok::Hex(dst) => {
                        let mut units: Vec<u16> =
                            dst.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
                        for code in lo..=hi.min(lo + 0xFFFF) {
                            map.insert(code, String::from_utf16_lossy(&units));
                            if let Some(last) = units.last_mut() {
                                *last = last.wrapping_add(1);
                            }
                        }
                        i += 3;
                    }
                    Tok::Open => {
                        let mut k = i + 3;
                        let mut code = lo;
                        while let Some(Tok::Hex(dst)) = toks.get(k) {
                            map.insert(code, utf16be(dst));
                            code += 1;
                            k += 1;
                        }
                        i = k + 1;
                    }
                    _ => i += 3,
                }
            }
            _ => i += 1,
        }
    }
    (map, two)
}

impl FontCodec {
    fn from_dict(doc: &Document, font: &Dictionary) -> Self {
        let type0 = font.get(b"Subtype").and_then(Object::as_name).map_or(false, |n| n == b"Type0");
        let mut codec = FontCodec { two_byte: type0, ..Default::default() };
        if let Ok(obj) = font.get(b"ToUnicode") {
            if let Ok((_, Object::Stream(s))) = doc.dereference(obj) {
                if let Ok(data) = s.get_plain_content() {
                    let (map, two) = parse_cmap(&data);
                    codec.to_uni = map;
                    if let Some(t) = two {
                        codec.two_byte = t;
                    }
                }
            }
        }
        codec.mask_code = codec.choose_mask_code();
        codec
    }

    /// 덮어쓸 코드: `*` 같은 비정보 글리프 → 없으면 아무 기호 → 최후엔 0(.notdef, 보이지 않음)
    fn choose_mask_code(&self) -> u32 {
        for want in ["*", "X", "x", "#", "-", "_", " "] {
            if let Some((&c, _)) = self.to_uni.iter().filter(|(_, v)| v.as_str() == want).min_by_key(|(c, _)| **c) {
                return c;
            }
        }
        if !self.two_byte {
            return b'*' as u32; // WinAnsi
        }
        self.to_uni
            .iter()
            .filter(|(_, v)| v.chars().count() == 1 && v.chars().all(|c| !c.is_alphanumeric() && !c.is_whitespace()))
            .map(|(&c, _)| c)
            .min()
            .unwrap_or(0)
    }

    fn step(&self) -> usize {
        if self.two_byte {
            2
        } else {
            1
        }
    }

    /// 문자열 바이트 → (유니코드, 바이트 위치)
    fn decode(&self, bytes: &[u8]) -> Vec<(String, usize)> {
        let step = self.step();
        let mut out = Vec::with_capacity(bytes.len() / step);
        let mut i = 0;
        while i + step <= bytes.len() {
            let code = be_code(&bytes[i..i + step]);
            let s = match self.to_uni.get(&code) {
                Some(s) => s.clone(),
                None if !self.two_byte => encoding_rs::WINDOWS_1252.decode_without_bom_handling(&bytes[i..i + 1]).0.into_owned(),
                None => "\u{FFFD}".into(),
            };
            out.push((s, i));
            i += step;
        }
        out
    }

    fn mask_bytes(&self) -> Vec<u8> {
        let b = self.mask_code.to_be_bytes();
        b[4 - self.step()..].to_vec()
    }
}

/// 리소스 사전의 /Font → (이름 → 폰트 번호)
fn fonts_of(doc: &Document, resources: Option<&Dictionary>, codecs: &mut Vec<FontCodec>) -> HashMap<Vec<u8>, usize> {
    let mut names = HashMap::new();
    let Some(res) = resources else { return names };
    let Ok(fonts) = res.get(b"Font").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_dict()) else {
        return names;
    };
    for (name, obj) in fonts.iter() {
        if let Ok((_, Object::Dictionary(fd))) = doc.dereference(obj) {
            codecs.push(FontCodec::from_dict(doc, fd));
            names.insert(name.clone(), codecs.len() - 1);
        }
    }
    names
}

// ───────────────────────────── 본문 텍스트 ─────────────────────────────

/// 글자 하나가 콘텐츠 스트림의 어디서 왔는가
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct CharRef {
    op: usize,
    operand: usize,
    arr: Option<usize>,
    byte: usize,
    font: usize,
}

/// 같은 글자들을 "조각을 붙인" 판과 "조각 사이를 띄운" 판 두 가지로 이어 붙인다.
#[derive(Default)]
struct TextBuild {
    joined: (String, Vec<Option<CharRef>>),
    spaced: (String, Vec<Option<CharRef>>),
}

impl TextBuild {
    fn push(&mut self, c: char, r: CharRef) {
        for (t, m) in [&mut self.joined, &mut self.spaced] {
            t.push(c);
            m.push(Some(r));
        }
    }
    /// 조각 경계 (같은 줄)
    fn soft(&mut self) {
        if !self.spaced.0.ends_with([' ', '\n']) && !self.spaced.0.is_empty() {
            self.spaced.0.push(' ');
            self.spaced.1.push(None);
        }
    }
    /// 줄 바꿈
    fn hard(&mut self) {
        for (t, m) in [&mut self.joined, &mut self.spaced] {
            if !t.is_empty() && !t.ends_with('\n') {
                t.push('\n');
                m.push(None);
            }
        }
    }
}

fn num(o: &Object) -> f32 {
    match o {
        Object::Integer(i) => *i as f32,
        Object::Real(r) => *r,
        _ => 0.0,
    }
}

/// 콘텐츠 스트림 하나 처리 → (새 스트림 데이터, 레코드). `font` 는 스트림 사이에 이어지는 현재 폰트.
fn process_content(
    data: &[u8],
    codecs: &[FontCodec],
    names: &HashMap<Vec<u8>, usize>,
    font: &mut usize,
    ctx: &Ctx,
) -> Option<(Vec<u8>, Vec<Record>)> {
    let mut content = Content::decode(data).ok()?;
    let mut tb = TextBuild::default();
    // 줄 판단: 텍스트 행렬의 y 를 추적해, 출력 시점에 직전 출력과 y 가 다르면 줄바꿈, 같으면 조각 경계
    let mut line_y = 0.0f32;
    let mut leading = 0.0f32;
    let mut last_shown_y: Option<f32> = None;
    let mut moved = false;

    for (oi, op) in content.operations.iter().enumerate() {
        let show = |tb: &mut TextBuild, s: &[u8], operand: usize, arr: Option<usize>, font: usize| {
            for (u, byte) in codecs[font].decode(s) {
                for c in u.chars() {
                    tb.push(c, CharRef { op: oi, operand, arr, byte, font });
                }
            }
        };
        let mut before_show = |tb: &mut TextBuild, y: f32, moved: &mut bool| {
            match last_shown_y {
                Some(ly) if (ly - y).abs() > 0.5 => tb.hard(),
                Some(_) if *moved => tb.soft(),
                _ => {}
            }
            last_shown_y = Some(y);
            *moved = false;
        };
        match op.operator.as_str() {
            "BT" => {
                line_y = 0.0;
                moved = true;
            }
            "TL" => leading = op.operands.first().map_or(0.0, num),
            "T*" => {
                line_y -= leading;
                moved = true;
            }
            "Tf" => {
                if let Some(Object::Name(n)) = op.operands.first() {
                    if let Some(&f) = names.get(n) {
                        *font = f;
                    }
                }
            }
            "Td" | "TD" => {
                let ty = op.operands.get(1).map_or(0.0, num);
                if op.operator == "TD" {
                    leading = -ty;
                }
                line_y += ty;
                moved = true;
            }
            "Tm" => {
                line_y = op.operands.get(5).map_or(0.0, num);
                moved = true;
            }
            "Tj" | "'" | "\"" => {
                if op.operator != "Tj" {
                    line_y -= leading;
                    moved = true;
                }
                before_show(&mut tb, line_y, &mut moved);
                let idx = op.operands.len().saturating_sub(1);
                if let Some(Object::String(s, _)) = op.operands.get(idx) {
                    show(&mut tb, s, idx, None, *font);
                }
            }
            "TJ" => {
                before_show(&mut tb, line_y, &mut moved);
                if let Some(Object::Array(items)) = op.operands.first() {
                    for (ai, it) in items.iter().enumerate() {
                        match it {
                            Object::String(s, _) => show(&mut tb, s, 0, Some(ai), *font),
                            o if num(o) < -200.0 => tb.soft(),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // 두 판 모두 검사, 같은 글자를 이미 처리했으면 중복 기록하지 않음
    let mut masked: HashSet<CharRef> = HashSet::new();
    let mut recs = Vec::new();
    for (text, map) in [&tb.joined, &tb.spaced] {
        for h in ctx.det.scan(text, Lang::None) {
            let one = [h];
            let m = char_mask(text, &one);
            let refs: Vec<CharRef> = m.iter().zip(map.iter()).filter_map(|(&on, r)| if on { *r } else { None }).collect();
            if refs.is_empty() || refs.iter().all(|r| masked.contains(r)) {
                continue;
            }
            masked.extend(refs);
            recs.extend(records(text, &one));
        }
    }
    if masked.is_empty() {
        return None;
    }

    // 글자 코드 교체
    for r in &masked {
        let op = &mut content.operations[r.op];
        let target = match (op.operands.get_mut(r.operand), r.arr) {
            (Some(Object::Array(a)), Some(ai)) => a.get_mut(ai),
            (o, None) => o,
            _ => None,
        };
        if let Some(Object::String(bytes, _)) = target {
            let code = codecs[r.font].mask_bytes();
            if r.byte + code.len() <= bytes.len() {
                bytes[r.byte..r.byte + code.len()].copy_from_slice(&code);
            }
        }
    }
    Some((content.encode().ok()?, recs))
}

// ───────────────────────────── 문자열 객체 ─────────────────────────────

fn decode_pdf_string(b: &[u8]) -> (String, bool) {
    if b.starts_with(&[0xFE, 0xFF]) {
        (utf16be(&b[2..]), true)
    } else {
        (b.iter().map(|&c| c as char).collect(), false)
    }
}

fn encode_pdf_string(s: &str, utf16: bool) -> Vec<u8> {
    if utf16 {
        let mut v = vec![0xFE, 0xFF];
        s.encode_utf16().for_each(|u| v.extend(u.to_be_bytes()));
        v
    } else {
        s.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect()
    }
}

/// 객체 안의 문자열을 재귀적으로 검사 (문서정보, 주석, 링크, 양식 값 …)
fn mask_strings(obj: &mut Object, ctx: &Ctx, recs: &mut Vec<Record>) {
    match obj {
        Object::String(bytes, _) => {
            let (text, utf16) = decode_pdf_string(bytes);
            // 이진 문자열(암호 해시 등)은 건드리지 않음
            if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
                return;
            }
            let hits = ctx.det.scan(&text, Lang::None);
            if hits.is_empty() {
                return;
            }
            let (masked, r) = crate::common::mask_text(&text, &hits);
            recs.extend(r);
            *bytes = encode_pdf_string(&masked, utf16);
        }
        Object::Array(a) => a.iter_mut().for_each(|o| mask_strings(o, ctx, recs)),
        Object::Dictionary(d) => d.iter_mut().for_each(|(_, o)| mask_strings(o, ctx, recs)),
        Object::Stream(s) => s.dict.iter_mut().for_each(|(_, o)| mask_strings(o, ctx, recs)),
        _ => {}
    }
}

// ───────────────────────────── 이미지 ─────────────────────────────

fn dict_name<'a>(d: &'a Dictionary, k: &[u8]) -> Option<&'a [u8]> {
    d.get(k).and_then(Object::as_name).ok()
}

fn filters(d: &Dictionary) -> Vec<Vec<u8>> {
    match d.get(b"Filter") {
        Ok(Object::Name(n)) => vec![n.clone()],
        Ok(Object::Array(a)) => a.iter().filter_map(|o| o.as_name().ok().map(|n| n.to_vec())).collect(),
        _ => vec![],
    }
}

/// 색 성분 수 (DeviceRGB=3, DeviceGray=1, ICCBased=N). 그 외(Indexed 등)는 None
fn components(doc: &Document, d: &Dictionary) -> Option<u32> {
    match d.get(b"ColorSpace").ok()? {
        Object::Name(n) if n == b"DeviceRGB" => Some(3),
        Object::Name(n) if n == b"DeviceGray" => Some(1),
        o => {
            let (_, cs) = doc.dereference(o).ok()?;
            let arr = cs.as_array().ok()?;
            if arr.first()?.as_name().ok()? != b"ICCBased" {
                return None;
            }
            let (_, s) = doc.dereference(arr.get(1)?).ok()?;
            let n = s.as_stream().ok()?.dict.get(b"N").ok()?.as_i64().ok()?;
            matches!(n, 1 | 3).then_some(n as u32)
        }
    }
}

fn ascii85(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut group = [0u8; 5];
    let mut n = 0;
    let data = input.strip_prefix(b"<~").unwrap_or(input);
    for &c in data {
        match c {
            b'~' => break,
            b'z' if n == 0 => out.extend([0u8; 4]),
            b'!'..=b'u' => {
                group[n] = c - b'!';
                n += 1;
                if n == 5 {
                    let v = group.iter().fold(0u64, |a, &d| a * 85 + d as u64);
                    out.extend((v as u32).to_be_bytes());
                    n = 0;
                }
            }
            c if c.is_ascii_whitespace() => {}
            _ => return None,
        }
    }
    if n > 1 {
        for g in group.iter_mut().skip(n) {
            *g = 84;
        }
        let v = group.iter().fold(0u64, |a, &d| a * 85 + d as u64);
        out.extend(&(v as u32).to_be_bytes()[..n - 1]);
    }
    Some(out)
}

/// 이미지 앞단 필터(ASCII85/ASCIIHex/Flate) 해제
fn decode_prefix(filters: &[Vec<u8>], content: &[u8]) -> Option<Vec<u8>> {
    let mut data = content.to_vec();
    for f in filters {
        data = match f.as_slice() {
            b"ASCII85Decode" | b"A85" => ascii85(&data)?,
            b"ASCIIHexDecode" | b"AHx" => {
                let hex: Vec<u8> = data.iter().copied().take_while(|&c| c != b'>').filter(u8::is_ascii_hexdigit).collect();
                hex.chunks(2)
                    .map(|c| {
                        let v = u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()?;
                        Some(if c.len() == 1 { v << 4 } else { v })
                    })
                    .collect::<Option<Vec<u8>>>()?
            }
            b"FlateDecode" | b"Fl" => {
                let mut out = Vec::new();
                std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&data[..]), &mut out).ok()?;
                out
            }
            _ => return None,
        };
    }
    Some(data)
}

/// 이미지 스트림 처리 → (새 데이터, 압축 해제 상태인지, 레코드)
fn process_image(doc: &Document, dict: &Dictionary, content: &[u8], ctx: &Ctx) -> Option<(Vec<u8>, bool, Vec<Record>)> {
    let f = filters(dict);
    let w = dict.get(b"Width").and_then(Object::as_i64).ok()? as u32;
    let h = dict.get(b"Height").and_then(Object::as_i64).ok()? as u32;
    if w < 32 || h < 16 {
        return None;
    }
    if f.last().map_or(false, |l| l == b"DCTDecode") {
        // [ASCII85Decode DCTDecode] 같은 체인은 앞단을 풀어 JPEG 를 꺼냄 → 저장 시 DCTDecode 단독
        let jpeg = decode_prefix(&f[..f.len() - 1], content)?;
        let o = raster::process(&jpeg, ImageFormat::Jpeg, ctx);
        return o.data.map(|d| (d, false, o.records));
    }
    if !f.is_empty() || dict.get(b"BitsPerComponent").and_then(Object::as_i64).ok()? != 8 {
        return None;
    }
    let n = components(doc, dict)?;
    if content.len() < (w * h * n) as usize {
        return None;
    }
    let mut img = RgbaImage::new(w, h);
    for (i, p) in img.pixels_mut().enumerate() {
        let s = &content[i * n as usize..];
        *p = if n == 3 { image::Rgba([s[0], s[1], s[2], 255]) } else { image::Rgba([s[0], s[0], s[0], 255]) };
    }
    let recs = raster::process_rgba(&mut img, ctx).ok()?;
    if recs.is_empty() {
        return None;
    }
    let mut raw = Vec::with_capacity((w * h * n) as usize);
    for p in img.pixels() {
        if n == 3 {
            raw.extend(&p.0[..3]);
        } else {
            raw.push(p.0[0]);
        }
    }
    Some((raw, true, recs))
}

// ───────────────────────────── 문서 전체 ─────────────────────────────

/// 페이지/Form XObject 의 콘텐츠 스트림 목록과 그 폰트
struct ContentJob {
    streams: Vec<ObjectId>,
    codecs: Vec<FontCodec>,
    names: HashMap<Vec<u8>, usize>,
}

fn collect_forms(doc: &Document, res: Option<&Dictionary>, seen: &mut HashSet<ObjectId>, jobs: &mut Vec<ContentJob>, inherit: &ContentJob, depth: u32) {
    let Some(res) = res else { return };
    let Ok(xo) = res.get(b"XObject").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_dict()) else { return };
    for (_, r) in xo.iter() {
        let Ok(id) = r.as_reference() else { continue };
        if depth > 4 || !seen.insert(id) {
            continue;
        }
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        if dict_name(&s.dict, b"Subtype") != Some(b"Form") {
            continue;
        }
        let own = s.dict.get(b"Resources").and_then(|o| doc.dereference(o)).and_then(|(_, o)| o.as_dict()).ok();
        let mut codecs = Vec::new();
        let names = fonts_of(doc, own, &mut codecs);
        let job = if names.is_empty() {
            ContentJob { streams: vec![id], codecs: inherit.codecs.clone(), names: inherit.names.clone() }
        } else {
            ContentJob { streams: vec![id], codecs, names }
        };
        collect_forms(doc, own, seen, jobs, &job, depth + 1);
        jobs.push(job);
    }
}

fn content_jobs(doc: &Document) -> Vec<ContentJob> {
    let mut jobs = Vec::new();
    let mut seen = HashSet::new();
    for (_, page) in doc.get_pages() {
        let mut codecs = Vec::new();
        let mut names = HashMap::new();
        let (own, parents) = doc.get_page_resources(page).unwrap_or((None, vec![]));
        let parent_dicts: Vec<&Dictionary> = parents.iter().filter_map(|id| doc.get_dictionary(*id).ok()).collect();
        // 상속 리소스 → 자기 리소스 순 (자기 것이 우선)
        for res in parent_dicts.iter().rev().copied().chain(own) {
            names.extend(fonts_of(doc, Some(res), &mut codecs));
        }
        let job = ContentJob { streams: doc.get_page_contents(page), codecs, names };
        for res in parent_dicts.iter().copied().chain(own) {
            collect_forms(doc, Some(res), &mut seen, &mut jobs, &job, 0);
        }
        seen.extend(job.streams.iter().copied());
        jobs.push(job);
    }
    jobs
}

fn is_font_or_cmap(dict: &Dictionary, content: &[u8]) -> bool {
    dict.has(b"Length1")
        || dict.has(b"Length2")
        || matches!(dict_name(dict, b"Subtype"), Some(b"Type1C" | b"CIDFontType0C" | b"OpenType"))
        || crate::common::find_bytes(&content[..content.len().min(4096)], b"begincmap").is_some()
}

pub fn process(bytes: &[u8], ctx: &Ctx) -> Outcome {
    match run(bytes, ctx) {
        Ok(o) => o,
        Err(e) => Outcome { notes: vec![format!("PDF 처리 실패: {e} — 일반 바이너리 검사로 대체")], ..super::binary::process(bytes, ctx) },
    }
}

fn run(bytes: &[u8], ctx: &Ctx) -> Result<Outcome, String> {
    let mut doc = Document::load_mem(bytes).map_err(|e| e.to_string())?;
    let mut out = Outcome::default();
    if doc.is_encrypted() {
        // 열기 암호 없는(권한 암호만 있는) PDF 는 풀어서 검사. 결과물은 암호화 없이 저장된다.
        doc.decrypt("").map_err(|_| "암호로 보호된 PDF - 검사 불가".to_string())?;
        doc.trailer.remove(b"Encrypt");
        out.notes.push("권한 암호 해제 후 검사 (결과물은 암호화되지 않음)".into());
    }
    doc.prune_objects();
    doc.decompress();

    let mut recs = Vec::new();

    // 1) 본문 텍스트
    let jobs = content_jobs(&doc);
    let mut content_ids = HashSet::new();
    for job in &jobs {
        let mut font = usize::MAX;
        let mut codecs = job.codecs.clone();
        codecs.push(FontCodec { mask_code: b'*' as u32, ..Default::default() }); // 폰트 미지정 시 기본(WinAnsi)
        let default_font = codecs.len() - 1;
        for &id in &job.streams {
            content_ids.insert(id);
            if font == usize::MAX {
                font = default_font;
            }
            let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
            let Ok(data) = s.get_plain_content() else { continue };
            if let Some((new, r)) = process_content(&data, &codecs, &job.names, &mut font, ctx) {
                if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
                    s.set_plain_content(new);
                }
                recs.extend(r);
            }
        }
    }

    // 2) 이미지 / 3) 텍스트 스트림·첨부파일
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    let mut updates: BTreeMap<ObjectId, (Vec<u8>, bool)> = BTreeMap::new();
    for id in ids {
        if content_ids.contains(&id) {
            continue;
        }
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        let subtype = dict_name(&s.dict, b"Subtype");
        if subtype == Some(b"Image") {
            if !ctx.det.opts.ocr {
                continue;
            }
            if let Some((data, plain, r)) = process_image(&doc, &s.dict, &s.content, ctx) {
                updates.insert(id, (data, plain));
                recs.extend(r);
            }
            continue;
        }
        if !filters(&s.dict).is_empty() || is_font_or_cmap(&s.dict, &s.content) {
            continue; // 해제 못 한 스트림 / 폰트 / CMap
        }
        if dict_name(&s.dict, b"Type") == Some(b"EmbeddedFile") {
            let o = process_entry("attachment", &s.content, &ctx.deeper());
            recs.extend(o.records);
            out.notes.extend(o.notes.into_iter().map(|n| format!("첨부파일: {n}")));
            if let Some(d) = o.data {
                updates.insert(id, (d, true));
            }
            continue;
        }
        if decode_text(&s.content).is_some() {
            let o = super::text::process("metadata.txt", &s.content, ctx);
            recs.extend(o.records);
            if let Some(d) = o.data {
                updates.insert(id, (d, true));
            }
        }
    }
    for (id, (data, plain)) in updates {
        if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
            if plain {
                s.set_plain_content(data);
            } else {
                // 이미지가 JPEG 로 재인코딩됨 → 필터를 DCTDecode 단독으로
                s.dict.remove(b"DecodeParms");
                s.dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
                s.set_content(data);
            }
        }
    }

    // 4) 문자열 객체 + 문서 정보(trailer /Info 는 객체로 존재)
    for obj in doc.objects.values_mut() {
        mask_strings(obj, ctx, &mut recs);
    }

    if recs.is_empty() && out.notes.is_empty() {
        return Ok(out); // 변경 없음 → 원본 그대로
    }
    doc.compress();
    let mut buf = Vec::with_capacity(bytes.len());
    doc.save_to(&mut buf).map_err(|e| e.to_string())?;
    out.data = Some(buf);
    out.records = recs;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a85() {
        assert_eq!(ascii85(b"<~87cURD]i,\"Ebo80~>").unwrap(), b"Hello World!");
    }

    #[test]
    fn cmap_parse() {
        let cm = b"1 begincodespacerange <0000> <FFFF> endcodespacerange
2 beginbfchar <0003> <0020> <0011> <002A> endbfchar
1 beginbfrange <0013> <001C> <0030> endbfrange
1 beginbfrange <0020> <0021> [<AC00> <AC01>] endbfrange";
        let (m, two) = parse_cmap(cm);
        assert_eq!(two, Some(true));
        assert_eq!(m[&0x13], "0");
        assert_eq!(m[&0x1C], "9");
        assert_eq!(m[&0x21], "각");
        let c = FontCodec { two_byte: true, to_uni: m, mask_code: 0 };
        assert_eq!(c.choose_mask_code(), 0x11);
    }
}

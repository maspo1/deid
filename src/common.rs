//! 공통 모듈 — 모든 기능이 재사용하는 타입과 유틸리티.
//!
//! * 정보 타입(`InfoType`)과 검출 결과(`Hit`, `Record`)
//! * 마스킹(치환) 규칙
//! * 텍스트 인코딩 판별/복원 (UTF-8, UTF-8 BOM, UTF-16, CP949)
//! * 엔트로피, 문자 경계 검사 등 보조 함수

use encoding_rs::EUC_KR;

/// 치환에 사용하는 문자
pub const MASK_CHAR: char = '*';

// ───────────────────────────── 정보 타입 ─────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InfoType {
    Phone,
    /// 국가번호(+)로 시작하는 국제 표기 전화번호 (+82 10-…, +1 …)
    IntlPhone,
    Card,
    Email,
    Rrn,
    Frn,
    Ip,
    /// 검사 항목 전용 (검출 결과의 타입은 Ip): 점 대신 다른 기호를 쓴 IP 를 표 검사·학습 모델로 찾는다
    IpDeep,
    Mac,
    Password,
    Cookie,
    Session,
    Comment,
    Custom(String),
}

impl InfoType {
    /// 내장 타입 전체 (CLI 토글용 키 순서)
    pub const BUILTIN: [InfoType; 13] = [
        InfoType::Phone,
        InfoType::IntlPhone,
        InfoType::Card,
        InfoType::Email,
        InfoType::Rrn,
        InfoType::Frn,
        InfoType::Ip,
        InfoType::IpDeep,
        InfoType::Mac,
        InfoType::Password,
        InfoType::Cookie,
        InfoType::Session,
        InfoType::Comment,
    ];

    /// 보고서에 기록되는 한글 이름
    pub fn label(&self) -> &str {
        match self {
            InfoType::Phone => "핸드폰번호",
            InfoType::IntlPhone => "해외핸드폰번호",
            InfoType::Card => "카드번호",
            InfoType::Email => "이메일주소",
            InfoType::Rrn => "주민등록번호",
            InfoType::Frn => "외국인등록번호",
            InfoType::Ip => "IP주소",
            InfoType::IpDeep => "IP주소(심층)",
            InfoType::Mac => "MAC주소",
            InfoType::Password => "패스워드",
            InfoType::Cookie => "쿠키",
            InfoType::Session => "세션 인증 값",
            InfoType::Comment => "소스코드 주석",
            InfoType::Custom(s) => s,
        }
    }

    /// CLI 옵션에서 쓰는 영문 키
    pub fn key(&self) -> &str {
        match self {
            InfoType::Phone => "phone",
            InfoType::IntlPhone => "intl-phone",
            InfoType::Card => "card",
            InfoType::Email => "email",
            InfoType::Rrn => "rrn",
            InfoType::Frn => "frn",
            InfoType::Ip => "ip",
            InfoType::IpDeep => "ip-deep",
            InfoType::Mac => "mac",
            InfoType::Password => "password",
            InfoType::Cookie => "cookie",
            InfoType::Session => "session",
            InfoType::Comment => "comment",
            InfoType::Custom(s) => s,
        }
    }

    pub fn from_key(k: &str) -> Option<InfoType> {
        Self::BUILTIN.iter().find(|t| t.key() == k).cloned()
    }

    /// 겹치는 검출이 있을 때의 우선순위 (클수록 우선)
    pub fn priority(&self) -> u8 {
        match self {
            InfoType::Custom(_) => 5,
            InfoType::Password | InfoType::Session | InfoType::Cookie => 4,
            InfoType::Rrn | InfoType::Frn | InfoType::Card => 3,
            InfoType::Email | InfoType::Phone | InfoType::IntlPhone => 2,
            InfoType::Ip | InfoType::IpDeep | InfoType::Mac => 1,
            InfoType::Comment => 0,
        }
    }
}

// ───────────────────────────── 검출 결과 ─────────────────────────────

/// 마스킹 방식
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskMode {
    /// 영숫자(한글 포함)만 치환, 구분자는 유지 (예: 010-****-****)
    Alnum,
    /// 공백을 제외한 모든 문자 치환 (패스워드, 토큰, 주석)
    NonSpace,
}

/// 텍스트 내 검출 위치 (바이트 오프셋, `start..end`)
#[derive(Clone, Debug)]
pub struct Hit {
    pub start: usize,
    pub end: usize,
    pub kind: InfoType,
    pub mode: MaskMode,
    /// 검출 확신도 (규칙 기반 = 1.0, 맥락 모델 = 확률)
    pub score: f32,
    /// 애매한 후보: 치환은 하되 보고서에 "검토필요" 로 표시 (대원칙: 애매하면 치환 후 사람이 검토)
    pub review: bool,
}

impl Hit {
    pub fn new(start: usize, end: usize, kind: InfoType, mode: MaskMode) -> Self {
        Hit { start, end, kind, mode, score: 1.0, review: false }
    }
    pub fn with_score(mut self, score: f32) -> Self {
        self.score = score;
        self
    }
    pub fn needs_review(mut self) -> Self {
        self.review = true;
        self
    }
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
    pub fn overlaps(&self, o: &Hit) -> bool {
        self.start < o.end && o.start < self.end
    }
}

/// 보고서 1행에 해당하는 처리 내역
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub original: String,
    pub masked: String,
    pub kind: InfoType,
    pub review: bool,
}

impl Record {
    /// 보고서용 정보타입 ("IP주소" / "IP주소(검토필요)")
    pub fn label(&self) -> String {
        if self.review {
            format!("{}(검토필요)", self.kind.label())
        } else {
            self.kind.label().to_string()
        }
    }
}

// ───────────────────────────── 마스킹 ─────────────────────────────

#[inline]
pub fn should_mask(c: char, mode: MaskMode) -> bool {
    match mode {
        MaskMode::Alnum => c.is_alphanumeric(),
        MaskMode::NonSpace => !c.is_whitespace(),
    }
}

/// 문자열 하나를 규칙대로 마스킹
pub fn mask_str(s: &str, mode: MaskMode) -> String {
    mask_str_with(s, mode, MASK_CHAR)
}

/// 치환 문자를 지정해 마스킹 (파일명 등 `*` 를 못 쓰는 곳)
pub fn mask_str_with(s: &str, mode: MaskMode, ch: char) -> String {
    s.chars().map(|c| if should_mask(c, mode) { ch } else { c }).collect()
}

/// hit 목록 → 문자 인덱스 단위 마스크 비트맵.
/// 텍스트 치환과 바이너리 제자리(in-place) 치환이 모두 이 함수를 공유한다.
pub fn char_mask(text: &str, hits: &[Hit]) -> Vec<bool> {
    let offs: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    let chars: Vec<char> = text.chars().collect();
    let mut mask = vec![false; chars.len()];
    for h in hits {
        let s = offs.partition_point(|&b| b < h.start);
        let e = offs.partition_point(|&b| b < h.end);
        for i in s..e {
            if should_mask(chars[i], h.mode) {
                mask[i] = true;
            }
        }
    }
    mask
}

/// 마스크 비트맵을 적용한 새 문자열
pub fn apply_mask(text: &str, mask: &[bool]) -> String {
    text.chars()
        .zip(mask.iter())
        .map(|(c, &m)| if m { MASK_CHAR } else { c })
        .collect()
}

/// hit 목록 → 보고서 레코드
pub fn records(text: &str, hits: &[Hit]) -> Vec<Record> {
    hits.iter()
        .filter_map(|h| {
            let s = text.get(h.start..h.end)?;
            let original = s.trim().to_string();
            if original.is_empty() {
                return None;
            }
            Some(Record { original, masked: mask_str(s, h.mode).trim().to_string(), kind: h.kind.clone(), review: h.review })
        })
        .collect()
}

/// 텍스트 전체 처리: (치환된 텍스트, 레코드)
pub fn mask_text(text: &str, hits: &[Hit]) -> (String, Vec<Record>) {
    let mask = char_mask(text, hits);
    (apply_mask(text, &mask), records(text, hits))
}

// ───────────────────────────── 인코딩 ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Cp949,
}

impl TextEncoding {
    pub fn label(&self) -> &'static str {
        match self {
            TextEncoding::Utf8 => "UTF-8",
            TextEncoding::Utf8Bom => "UTF-8(BOM)",
            TextEncoding::Utf16Le => "UTF-16LE",
            TextEncoding::Utf16Be => "UTF-16BE",
            TextEncoding::Cp949 => "CP949",
        }
    }
}

/// 제어문자 비율로 "사람이 읽는 텍스트"인지 판단
fn looks_textual(s: &str) -> bool {
    let total = s.len().max(1);
    let bad = s
        .bytes()
        .filter(|&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0c | 0x1b))
        .count();
    bad * 100 <= total // 1% 이하
}

fn decode_utf16(bytes: &[u8], le: bool) -> Option<String> {
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
        .collect();
    String::from_utf16(&units).ok()
}

/// 바이트열이 텍스트라면 (문자열, 원래 인코딩)을 반환
pub fn decode_text(bytes: &[u8]) -> Option<(String, TextEncoding)> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return std::str::from_utf8(rest).ok().map(|s| (s.to_string(), TextEncoding::Utf8Bom));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16(rest, true).map(|s| (s, TextEncoding::Utf16Le));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, false).map(|s| (s, TextEncoding::Utf16Be));
    }
    if bytes.contains(&0) {
        return None;
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return looks_textual(s).then(|| (s.to_string(), TextEncoding::Utf8));
    }
    // CP949(EUC-KR 확장) — 손실 없이 왕복 변환될 때만 텍스트로 인정
    let s = EUC_KR.decode_without_bom_handling_and_without_replacement(bytes)?;
    if !looks_textual(&s) {
        return None;
    }
    let (back, _, unmappable) = EUC_KR.encode(&s);
    if unmappable || back.as_ref() != bytes {
        return None;
    }
    Some((s.into_owned(), TextEncoding::Cp949))
}

/// 원래 인코딩으로 다시 바이트열 생성
pub fn encode_text(s: &str, enc: TextEncoding) -> Vec<u8> {
    match enc {
        TextEncoding::Utf8 => s.as_bytes().to_vec(),
        TextEncoding::Utf8Bom => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend_from_slice(s.as_bytes());
            v
        }
        TextEncoding::Utf16Le => {
            let mut v = vec![0xFF, 0xFE];
            s.encode_utf16().for_each(|u| v.extend_from_slice(&u.to_le_bytes()));
            v
        }
        TextEncoding::Utf16Be => {
            let mut v = vec![0xFE, 0xFF];
            s.encode_utf16().for_each(|u| v.extend_from_slice(&u.to_be_bytes()));
            v
        }
        TextEncoding::Cp949 => EUC_KR.encode(s).0.into_owned(),
    }
}

// ───────────────────────────── 보조 함수 ─────────────────────────────

/// 섀넌 엔트로피 (bit/문자)
pub fn shannon_entropy(s: &str) -> f32 {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return 0.0;
    }
    let mut counts = std::collections::HashMap::new();
    for c in &chars {
        *counts.entry(*c).or_insert(0usize) += 1;
    }
    let n = chars.len() as f32;
    counts.values().map(|&c| {
        let p = c as f32 / n;
        -p * p.log2()
    }).sum()
}

/// 문자 종류(소문자/대문자/숫자/기호) 개수
pub fn char_class_count(s: &str) -> usize {
    let (mut lo, mut up, mut dg, mut sy) = (false, false, false, false);
    for c in s.chars() {
        if c.is_ascii_lowercase() {
            lo = true
        } else if c.is_ascii_uppercase() {
            up = true
        } else if c.is_ascii_digit() {
            dg = true
        } else if !c.is_whitespace() {
            sy = true
        }
    }
    [lo, up, dg, sy].iter().filter(|&&b| b).count()
}

#[inline]
pub fn char_before(text: &str, i: usize) -> Option<char> {
    text.get(..i)?.chars().next_back()
}

#[inline]
pub fn char_after(text: &str, i: usize) -> Option<char> {
    text.get(i..)?.chars().next()
}

/// i 이하에서 가장 가까운 문자 경계
pub fn floor_boundary(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// i 이상에서 가장 가까운 문자 경계
pub fn ceil_boundary(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// [start-before, end+after] 주변 문맥 (소문자)
pub fn context_window(text: &str, start: usize, end: usize, before: usize, after: usize) -> String {
    let s = floor_boundary(text, start.saturating_sub(before));
    let e = ceil_boundary(text, end.saturating_add(after));
    text[s..e].to_lowercase()
}

/// 바이트열에서 부분열 찾기
pub fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// 겹치는 hit 정리: 우선순위 → 확신도 → 길이 순으로 채택하고 겹치는 나머지는 버린다.
pub fn resolve_overlaps(mut hits: Vec<Hit>) -> Vec<Hit> {
    hits.sort_by(|a, b| {
        b.kind
            .priority()
            .cmp(&a.kind.priority())
            .then(b.score.total_cmp(&a.score))
            .then(b.len().cmp(&a.len()))
            .then(a.start.cmp(&b.start))
    });
    // 남긴 것끼리는 겹치지 않으므로 시작 위치로 정렬해 두면, h.end 보다 앞에서 시작하는 마지막 하나만 보면 된다
    // (그것이 h 와 안 겹치면 그 앞의 것들은 더 일찍 끝난다) → 검출 수가 많아도 O(n log n)
    let mut kept: std::collections::BTreeMap<usize, Hit> = std::collections::BTreeMap::new();
    for h in hits {
        if h.is_empty() {
            continue;
        }
        let clash = kept.range(..h.end).next_back().map_or(false, |(_, k)| k.overlaps(&h));
        if !clash {
            kept.insert(h.start, h);
        }
    }
    kept.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 정렬 색인 방식이 모든 쌍을 비교하던 방식과 같은 결과인가 (무작위 구간 2만 묶음)
    #[test]
    fn resolve_overlaps_matches_quadratic() {
        fn reference(mut hits: Vec<Hit>) -> Vec<Hit> {
            hits.sort_by(|a, b| {
                b.kind.priority().cmp(&a.kind.priority()).then(b.score.total_cmp(&a.score)).then(b.len().cmp(&a.len())).then(a.start.cmp(&b.start))
            });
            let mut kept: Vec<Hit> = Vec::new();
            for h in hits {
                if !h.is_empty() && !kept.iter().any(|k| k.overlaps(&h)) {
                    kept.push(h);
                }
            }
            kept.sort_by_key(|h| h.start);
            kept
        }
        let mut x = 99u64;
        let mut rnd = |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        let kinds = [InfoType::Phone, InfoType::Ip, InfoType::Card, InfoType::Password, InfoType::Email];
        for _ in 0..20_000 {
            let n = rnd(12);
            let hits: Vec<Hit> = (0..n)
                .map(|_| {
                    let s = rnd(60) as usize;
                    let e = s + rnd(15) as usize;
                    Hit::new(s, e, kinds[rnd(5) as usize].clone(), MaskMode::Alnum).with_score(rnd(3) as f32 / 2.0)
                })
                .collect();
            let key = |v: Vec<Hit>| v.into_iter().map(|h| (h.start, h.end, h.kind, h.score.to_bits())).collect::<Vec<_>>();
            assert_eq!(key(resolve_overlaps(hits.clone())), key(reference(hits)));
        }
    }

    #[test]
    fn mask_keeps_separators() {
        assert_eq!(mask_str("010-1111-2222", MaskMode::Alnum), "***-****-****");
        assert_eq!(mask_str("192.168.77.1", MaskMode::Alnum), "***.***.**.*");
    }

    #[test]
    fn cp949_roundtrip() {
        let (bytes, _, _) = EUC_KR.encode("안녕하세요 010-1234-5678");
        let (s, enc) = decode_text(&bytes).unwrap();
        assert_eq!(enc, TextEncoding::Cp949);
        assert_eq!(encode_text(&s, enc), bytes.as_ref());
    }

    #[test]
    fn mask_text_multibyte() {
        let t = "번호 010-1234-5678 끝";
        let s = t.find("010").unwrap();
        let hits = vec![Hit::new(s, s + 13, InfoType::Phone, MaskMode::Alnum)];
        let (out, recs) = mask_text(t, &hits);
        assert_eq!(out, "번호 ***-****-**** 끝");
        assert_eq!(recs[0].original, "010-1234-5678");
    }
}

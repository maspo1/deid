//! 1단계 — 정형 데이터 기계적 탐색 (정규식).
//! 각 후보는 경계 검사 후 `dict`(2단계)를 통과해야 Hit 이 된다.

use super::dict;
use crate::common::{char_after, char_before, Hit, InfoType, MaskMode};
use regex::Regex;

pub struct Patterns {
    phone: Regex,
    card: Regex,
    card_amex: Regex,
    email: Regex,
    rrn: Regex,
    ipv4: Regex,
    mac: Regex,
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("내장 정규식 오류")
}

/// 숫자 앞뒤 경계: 인접 문자가 숫자가 아니어야 하며, "-숫자" 로 이어지지도 않아야 한다.
pub(super) fn digit_bounded(text: &str, s: usize, e: usize) -> bool {
    let before_ok = match char_before(text, s) {
        Some(c) if c.is_ascii_digit() => false,
        Some('-') | Some('.') => !char_before(text, s - 1).map_or(false, |c| c.is_ascii_digit()),
        _ => true,
    };
    let after_ok = match char_after(text, e) {
        Some(c) if c.is_ascii_digit() => false,
        Some('-') | Some('.') => !char_after(text, e + 1).map_or(false, |c| c.is_ascii_digit()),
        _ => true,
    };
    before_ok && after_ok
}

/// 영문이 바로 붙은 숫자열은 코드·해시(0x0103…, ab0103…x)다. 단 전화 키워드(tel, hp …)가 붙은 경우는 번호.
pub(super) fn glued_to_code(text: &str, s: usize, e: usize) -> bool {
    if char_after(text, e).map_or(false, |c| c.is_ascii_alphabetic()) {
        return true;
    }
    let word: String = text[..s].chars().rev().take_while(|c| c.is_ascii_alphabetic()).collect::<Vec<_>>().into_iter().rev().collect();
    let word = word.to_ascii_lowercase();
    !word.is_empty() && !["tel", "hp", "phone", "mobile", "cell", "call", "fax", "no"].iter().any(|k| word.ends_with(k))
}

/// 카드 표기 배열 (구분자 위치 정답 행렬)
fn card_groups_ok(s: &str) -> bool {
    let g: Vec<usize> = s.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).map(str::len).collect();
    dict::card_layout(&g)
}

/// 구분자 일관성: 숫자가 아닌 문자들이 모두 같은 문자여야 함 (없어도 됨)
fn uniform_separators(s: &str, expected_groups: usize) -> bool {
    let seps: Vec<char> = s.chars().filter(|c| !c.is_ascii_digit()).collect();
    seps.is_empty() || (seps.len() == expected_groups - 1 && seps.iter().all(|&c| c == seps[0]))
}

/// 바로 앞 단어가 버전 표기인가 (version, ver, v, build, rev, 버전)
pub(super) fn preceded_by_version_word(text: &str, s: usize) -> bool {
    let back = crate::common::context_window(text, s, s, 24, 0);
    let back = back.trim_end_matches([' ', ':', '=', '\t', '(', '"', '\'', '-', '_']);
    let word: String =
        back.chars().rev().take_while(|c| c.is_alphabetic() || *c == '.').collect::<Vec<_>>().into_iter().rev().collect();
    let w = word.trim_end_matches('.');
    matches!(w, "v" | "ver" | "version" | "build" | "rev" | "revision" | "release" | "버전" | "빌드") || w.starts_with("version")
}

/// 직전 40자 안에 IP 를 암시하는 맥락이 있는가
fn ip_context(text: &str, s: usize) -> bool {
    const KEYS: &[&str] = &[
        "ip", "host", "server", "addr", "gateway", "dns", "ping", "proxy", "://", "주소", "서버", "접속", "호스트",
        "아이피", "게이트웨이",
    ];
    let back = crate::common::context_window(text, s, s, 40, 0);
    KEYS.iter().any(|k| back.contains(k))
}

/// 직전 30자 안에 카드 맥락이 있는가
pub(super) fn card_context(text: &str, s: usize) -> bool {
    let back = crate::common::context_window(text, s, s, 30, 0);
    ["카드", "card", "결제", "visa", "master"].iter().any(|k| back.contains(k))
}

impl Patterns {
    pub fn new() -> Self {
        Patterns {
            phone: re(r"(\+82[-. ]?)?(0?1[0-9])([-. ]?)([0-9]{3,4})([-. ]?)([0-9]{4})"),
            card: re(r"[0-9]{4}[- ]?[0-9]{4}[- ]?[0-9]{4}[- ]?[0-9]{1,4}"),
            card_amex: re(r"3[0-9]{3}[- ]?[0-9]{6}[- ]?[0-9]{4,5}"),
            email: re(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)*\.[A-Za-z]{2,24}"),
            rrn: re(r"([0-9]{6})([- ]?)([0-9]{7})"),
            // 점으로 이어진 숫자열 전체(4묶음 이상)를 한 덩어리로 보고 구조로 판단
            ipv4: re(r"[0-9]+(?:\.[0-9]+){3,}"),
            mac: re(r"[0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}"),
        }
    }

    pub fn phones(&self, text: &str, out: &mut Vec<Hit>) {
        for c in self.phone.captures_iter(text) {
            let m = c.get(0).unwrap();
            let intl = c.get(1).is_some();
            let prefix = &c[2];
            // 국제표기(+82)가 아니면 반드시 0으로 시작
            if !intl && !prefix.starts_with('0') {
                continue;
            }
            if &c[3] != &c[5] {
                continue; // 구분자 불일치 (010-1234 5678)
            }
            if !digit_bounded(text, m.start(), m.end()) || glued_to_code(text, m.start(), m.end()) {
                continue;
            }
            if dict::valid_mobile(prefix, &c[4], &c[6]) {
                out.push(Hit::new(m.start(), m.end(), InfoType::Phone, MaskMode::Alnum));
            }
        }
    }

    /// 기계적 치환 — 구분자가 바뀌거나 빠져도 **누가 봐도** 휴대폰 번호인 것 (`0105678-1234`, `010_5678_1234`).
    /// 010 국번은 항상 11자리이고 다른 숫자 형식과 겹칠 일이 거의 없어 확률 추론 없이 규칙으로 처리한다.
    /// 덩어리 전체가 번호여야 하고(앞뒤로 다른 숫자 묶음이 이어지지 않음), 영문이 바로 붙으면 제외(코드·해시).
    pub fn phones_mechanical(&self, text: &str, out: &mut Vec<Hit>) {
        self.phones_mechanical_in(text, &super::ml::features::candidates(text), out)
    }

    /// 미리 만든 숫자 묶음 후보로 (Detector 가 ML 과 후보를 함께 씀)
    pub fn phones_mechanical_in(&self, text: &str, cands: &[super::ml::features::Candidate], out: &mut Vec<Hit>) {
        for c in cands {
            if c.sep_before.is_some() || c.sep_after.is_some() || c.groups.len() > 4 {
                continue;
            }
            let d = c.digits();
            if d.len() != 11 || !d.starts_with("010") {
                continue;
            }
            let alpha = |x: Option<char>| x.map_or(false, |x| x.is_ascii_alphabetic());
            if alpha(char_before(text, c.start)) || alpha(char_after(text, c.end)) {
                continue;
            }
            if dict::valid_mobile(&d[..3], &d[3..7], &d[7..]) {
                out.push(Hit::new(c.start, c.end, InfoType::Phone, MaskMode::Alnum));
            }
        }
    }

    pub fn cards(&self, text: &str, out: &mut Vec<Hit>) {
        for (re, groups) in [(&self.card, 4usize), (&self.card_amex, 3usize)] {
            for m in re.find_iter(text) {
                let s = m.as_str();
                if !uniform_separators(s, groups) || !digit_bounded(text, m.start(), m.end()) || !card_groups_ok(s) {
                    continue;
                }
                let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
                let hit = Hit::new(m.start(), m.end(), InfoType::Card, MaskMode::Alnum);
                if dict::valid_card(&digits) {
                    // 붙여 쓴 숫자열은 계좌·주문·ID 번호와 겹친다 → 카드 맥락이 없으면 치환 + 검토필요
                    let bare = s.len() == digits.len();
                    out.push(if bare && !card_context(text, m.start()) { hit.needs_review() } else { hit });
                } else if s.len() > digits.len()
                    && (13..=19).contains(&digits.len())
                    && dict::valid_card_iin(&digits)
                    && card_context(text, m.start())
                {
                    out.push(hit.needs_review());
                }
            }
        }
    }

    pub fn emails(&self, text: &str, out: &mut Vec<Hit>) {
        for m in self.email.find_iter(text) {
            // 도메인 뒤에 영숫자가 바로 붙으면 (예: 한글 조사 제외) 잘린 매칭
            if char_after(text, m.end()).map_or(false, |c| c.is_ascii_alphanumeric()) {
                continue;
            }
            if dict::valid_email(m.as_str()) {
                out.push(Hit::new(m.start(), m.end(), InfoType::Email, MaskMode::Alnum));
            }
        }
    }

    /// 주민등록번호 + 외국인등록번호
    pub fn rrns(&self, text: &str, out: &mut Vec<Hit>) {
        for c in self.rrn.captures_iter(text) {
            let m = c.get(0).unwrap();
            if !digit_bounded(text, m.start(), m.end()) {
                continue;
            }
            let has_sep = !c[2].is_empty();
            if let Some(kind) = dict::rrn_kind(&c[1], &c[3], has_sep) {
                // 보통의 표기(YYMMDD-NNNNNNN)가 아니면 의심 → 치환 + 검토필요
                let hit = Hit::new(m.start(), m.end(), kind, MaskMode::Alnum);
                out.push(if &c[2] == "-" { hit } else { hit.needs_review() });
            }
        }
    }

    /// IPv4 — 단어가 아니라 숫자열의 **구조**로 판단한다.
    ///
    /// | 구조 | 예 | 처리 |
    /// |---|---|---|
    /// | 영문이 숫자열에 붙음 | `2.3.15.6.7x8.11.20.13x`, `v1.2.3.4` | 좌표/코드 표기 → 제외 |
    /// | 4묶음, 유효 | `30.21.2.1`, `10.0.0.1:8080` | IP |
    /// | 4묶음 + 끝맺는 점 | `2.3.56.8.` | 좌표일 수도, 문장 끝 IP 일 수도 → 맥락 없으면 치환+검토 |
    /// | 5묶음 이상, 첫 값 0/1/2 | `1.2.840.113549.1` | OID 표기 → 제외 |
    /// | 5묶음 이상, 유효한 IP 4묶음 포함 | `33.100.20.1.3` | IP 범위가 애매 → 전체 치환+검토 |
    pub fn ipv4s(&self, text: &str, out: &mut Vec<Hit>) {
        for m in self.ipv4.find_iter(text) {
            ipv4_at(text, m.start(), m.end(), out);
        }
    }

    pub fn macs(&self, text: &str, out: &mut Vec<Hit>) {
        for m in self.mac.find_iter(text) {
            let near = |c: Option<char>| c.map_or(false, |c| c.is_ascii_hexdigit() || c == ':' || c == '-');
            if near(char_before(text, m.start())) || near(char_after(text, m.end())) {
                continue;
            }
            if dict::valid_mac(m.as_str()) {
                out.push(Hit::new(m.start(), m.end(), InfoType::Mac, MaskMode::Alnum));
            }
        }
    }
}

/// 점 IP 가 다른 숫자 덩어리에 기호로 붙어 있는가 (`206*203_554$51~126.165.8.187$188.3068`).
/// 레코드 구분자(, ; | / 공백 괄호 따옴표 =), 포트(:), 다른 IP 와 이은 범위·목록(10.0.0.1~10.0.0.9)은 떼어낼 수 있으므로 아니다.
fn glued_in_blob(text: &str, s: usize, e: usize) -> bool {
    const SEP_OK: &str = ",;|/[](){}<>，、\"'=:";
    let quad = |t: &str| {
        let g: Vec<&str> = t.split('.').collect();
        g.len() == 4 && g.iter().all(|x| !x.is_empty() && x.len() <= 3)
    };
    let glue = |c: char| !c.is_alphanumeric() && !c.is_whitespace() && !SEP_OK.contains(c);
    if let Some(c) = char_before(text, s).filter(|&c| glue(c)) {
        let head = &text[..s - c.len_utf8()];
        let tok: String = head.chars().rev().take_while(|x| x.is_ascii_digit() || *x == '.').collect::<Vec<_>>().into_iter().rev().collect();
        if tok.ends_with(|x: char| x.is_ascii_digit()) && !quad(tok.trim_start_matches('.')) {
            return true;
        }
    }
    if let Some(c) = char_after(text, e).filter(|&c| glue(c)) {
        let tail = &text[e + c.len_utf8()..];
        let tok: String = tail.chars().take_while(|x| x.is_ascii_digit() || *x == '.').collect();
        if tok.starts_with(|x: char| x.is_ascii_digit()) && !quad(tok.trim_end_matches('.')) {
            return true;
        }
    }
    false
}

/// 점으로 이어진 숫자열 하나(`text[s..e]`, 4묶음 이상)를 구조 규칙으로 판단 — 정규식 경로와 정답 행렬 경로 공용
pub(super) fn ipv4_at(text: &str, s: usize, e: usize, out: &mut Vec<Hit>) {
    let before = char_before(text, s);
    let after = char_after(text, e);
    let letter = |c: Option<char>| c.map_or(false, |c| c.is_ascii_alphabetic());
    if letter(before) || letter(after) {
        return;
    }
    // 글자로 끊긴 점 사슬의 뒷부분(21.0.ㅇ.88.11.25.1) — 어디까지가 주소인지 하나의 규칙으로 복원할 수 없다
    if before == Some('.') && char_before(text, s - 1).map_or(false, |c| c.is_alphabetic()) {
        return;
    }
    // 숫자 덩어리에 기호로 붙은 점 사슬 — 하나의 규칙으로 떼어낼 수 없다
    if glued_in_blob(text, s, e) {
        return;
    }
    if preceded_by_version_word(text, s) {
        return;
    }
    let groups: Vec<&str> = text[s..e].split('.').collect();
    let valid = |w: &[&str]| {
        let ip = w.join(".");
        dict::valid_ipv4(&ip) && !dict::ipv4_is_trivial(&ip)
    };
    let hit = Hit::new(s, e, InfoType::Ip, MaskMode::Alnum);
    if groups.len() == 4 {
        if !valid(&groups) {
            return;
        }
        // 끝맺는 점(뒤에 확장자 같은 글자가 이어지지 않음) 또는 앞의 점 → 좌표 표기와 구분 불가
        let terminated = after == Some('.') && !char_after(text, e + 1).map_or(false, |c| c.is_alphanumeric());
        let ambiguous = (terminated || before == Some('.')) && !ip_context(text, s);
        out.push(if ambiguous { hit.needs_review() } else { hit });
    } else {
        if matches!(groups[0], "0" | "1" | "2") {
            return; // OID (첫 arc 는 0/1/2 만 존재)
        }
        // 6묶음 이상이거나 뒤에 `.글자` 가 이어지는 사슬(11.7.8.5.122.90.3.1.7.x)은 절·버전·식별자 번호일 가능성이 크다
        // → IP 맥락이 있을 때만 검토. 5묶음(33.100.20.1.3)은 잘못 입력한 IP 일 수 있어 그대로 검토
        let dotted_word = after == Some('.') && char_after(text, e + 1).map_or(false, |c| c.is_alphabetic());
        if (groups.len() >= 6 || dotted_word) && !ip_context(text, s) {
            return;
        }
        if groups.windows(4).any(valid) {
            out.push(hit.needs_review());
        }
    }
}

impl Default for Patterns {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: impl Fn(&Patterns, &str, &mut Vec<Hit>), t: &str) -> Vec<String> {
        let p = Patterns::new();
        let mut v = Vec::new();
        f(&p, t, &mut v);
        v.iter().map(|h| t[h.start..h.end].to_string()).collect()
    }

    #[test]
    fn phone() {
        assert_eq!(run(Patterns::phones, "연락처: 010-1111-2222 입니다"), vec!["010-1111-2222"]);
        assert_eq!(run(Patterns::phones, "+82-10-1111-2222"), vec!["+82-10-1111-2222"]);
        assert_eq!(run(Patterns::phones, "01011112222"), vec!["01011112222"]);
        assert!(run(Patterns::phones, "999-1111-2222").is_empty());
        assert!(run(Patterns::phones, "id=2010111122222").is_empty());
    }

    #[test]
    fn ip() {
        assert_eq!(run(Patterns::ipv4s, "host 192.168.77.1:8080"), vec!["192.168.77.1"]);
        assert!(run(Patterns::ipv4s, "300.1.1.1").is_empty());
        assert!(run(Patterns::ipv4s, "version 1.0.0.1").is_empty());
        assert!(run(Patterns::ipv4s, "v1.0.0.1").is_empty());
        assert_eq!(run(Patterns::ipv4s, "Server IP 192.168.77.1"), vec!["192.168.77.1"]);
        assert_eq!(run(Patterns::ipv4s, "server=10.1.2.3"), vec!["10.1.2.3"]);
        assert!(run(Patterns::ipv4s, "oid 1.2.840.113549.1").is_empty());
        assert!(run(Patterns::ipv4s, "1.3.6.1.4.1.311.10.3.1").is_empty());
        // 좌표 표기: 영문이 숫자열에 붙음
        assert!(run(Patterns::ipv4s, "<p>2.3.15.6.7x8.11.20.13x</p>").is_empty());
        assert!(run(Patterns::ipv4s, "pos 3x8.11.20.13y").is_empty());
        // 애매: 치환 + 검토
        let p = Patterns::new();
        for (t, want) in [("<table>33.100.20.1.3</table>", "33.100.20.1.3"), ("2.3.56.8.", "2.3.56.8")] {
            let mut v = Vec::new();
            p.ipv4s(t, &mut v);
            assert_eq!(v.len(), 1, "{t}");
            assert_eq!(&t[v[0].start..v[0].end], want);
            assert!(v[0].review, "{t}");
        }
        // 맥락이 있으면 문장 끝 점은 확정
        let mut v = Vec::new();
        p.ipv4s("서버 주소는 10.20.30.40.", &mut v);
        assert!(v.len() == 1 && !v[0].review);
        // 파일명 확장자는 끝맺는 점이 아님
        let mut v = Vec::new();
        p.ipv4s("dump_10.20.30.40.log", &mut v);
        assert!(v.len() == 1 && !v[0].review);
    }

    #[test]
    fn rrn() {
        assert_eq!(run(Patterns::rrns, "주민번호 881111-1234567"), vec!["881111-1234567"]);
        assert!(run(Patterns::rrns, "881711-1234567").is_empty());
    }

    #[test]
    fn card() {
        assert_eq!(run(Patterns::cards, "카드 4111-1111-1111-1111 결제"), vec!["4111-1111-1111-1111"]);
        assert!(run(Patterns::cards, "4111-1111 1111-1111").is_empty());
        // Luhn 불일치: 카드 맥락이 있을 때만 치환+검토
        assert!(run(Patterns::cards, "주문 4111-1111-1111-1112").is_empty());
        let p = Patterns::new();
        let mut v = Vec::new();
        p.cards("카드번호 4111-1111-1111-1112", &mut v);
        assert!(v.len() == 1 && v[0].review);
    }

    #[test]
    fn mac() {
        assert_eq!(run(Patterns::macs, "mac=00:1A:2B:3C:4D:5E"), vec!["00:1A:2B:3C:4D:5E"]);
    }
}

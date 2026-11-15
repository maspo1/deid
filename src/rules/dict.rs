//! 2단계 — 사전 규칙(Database Dict) 검사.
//!
//! 1단계 정규식이 "형태"만 맞춘 후보를, 실제 존재할 수 있는 값인지 규칙으로 걸러낸다.
//! 예) IP 옥텟 ≤ 255, 휴대폰 국번 010/011/016~019, 주민번호 생년월일·성별코드 유효성.

use crate::common::InfoType;

// ───────────── IP 주소 ─────────────

/// 각 옥텟 0~255, 선행 0 금지
pub fn valid_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.bytes().all(|b| b.is_ascii_digit())
                && !(p.len() > 1 && p.starts_with('0'))
                && p.parse::<u16>().map(|v| v <= 255).unwrap_or(false)
        })
}

/// 식별 의미가 없는 IP (미지정/루프백/브로드캐스트/서브넷 마스크)
pub fn ipv4_is_trivial(s: &str) -> bool {
    const MASK_OCTETS: [&str; 9] = ["0", "128", "192", "224", "240", "248", "252", "254", "255"];
    if matches!(s, "0.0.0.0" | "127.0.0.1" | "255.255.255.255") {
        return true;
    }
    // 255.x.x.x 형태의 넷마스크
    let parts: Vec<&str> = s.split('.').collect();
    parts[0] == "255" && parts.iter().all(|p| MASK_OCTETS.contains(p))
}

// ───────────── 휴대폰 번호 ─────────────

/// prefix: "010" 또는 "10"(+82 표기), mid, last
pub fn valid_mobile(prefix: &str, mid: &str, last: &str) -> bool {
    let p = prefix.trim_start_matches('0');
    // 국번: 10, 11, 16, 17, 18, 19 만 존재
    if !matches!(p, "10" | "11" | "16" | "17" | "18" | "19") {
        return false;
    }
    if last.len() != 4 || mid.starts_with('0') {
        return false;
    }
    match p {
        "10" => mid.len() == 4,
        _ => mid.len() == 3 || mid.len() == 4,
    }
}

// ───────────── 카드 번호 ─────────────

pub fn luhn(digits: &str) -> bool {
    let mut sum = 0u32;
    for (i, b) in digits.bytes().rev().enumerate() {
        let mut d = (b - b'0') as u32;
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum % 10 == 0
}

/// 발급사 식별번호(IIN) 범위
pub fn valid_card_iin(d: &str) -> bool {
    let n = d.len();
    let p2: u32 = d[..2].parse().unwrap_or(0);
    let p4: u32 = d[..4].parse().unwrap_or(0);
    match d.as_bytes()[0] {
        b'4' => n == 13 || n == 16 || n == 19,                       // VISA
        b'5' => (51..=55).contains(&p2) && n == 16,                  // Master
        b'2' => (2221..=2720).contains(&p4) && n == 16,              // Master(2-series)
        b'3' => match p2 {
            34 | 37 => n == 15,                                      // AMEX
            35 => (16..=19).contains(&n),                            // JCB
            30 | 36 | 38 | 39 => (14..=16).contains(&n),             // Diners
            _ => false,
        },
        b'6' => (16..=19).contains(&n),                              // Discover/UnionPay
        b'9' => n == 16,                                             // 국내 전용(BC 등)
        _ => false,
    }
}

pub fn valid_card(digits: &str) -> bool {
    (13..=19).contains(&digits.len()) && valid_card_iin(digits) && luhn(digits)
}

// ───────────── 주민등록번호 / 외국인등록번호 ─────────────

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

fn current_year() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    1970 + (secs / 31_556_952) as u32
}

/// 주민번호 체크섬 (내국인/외국인 공식 모두 허용)
pub fn rrn_checksum(d13: &str) -> bool {
    const W: [u32; 12] = [2, 3, 4, 5, 6, 7, 8, 9, 2, 3, 4, 5];
    let b = d13.as_bytes();
    let sum: u32 = (0..12).map(|i| (b[i] - b'0') as u32 * W[i]).sum();
    let last = (b[12] - b'0') as u32;
    (11 - sum % 11) % 10 == last || (13 - sum % 11) % 10 == last
}

/// 앞 6자리(YYMMDD) + 뒤 7자리 검사.
/// 구분자(-)가 없으면 오탐 방지를 위해 체크섬까지 요구한다.
/// (2020.10 이후 발급 번호는 체크섬이 없으므로 구분자가 있으면 날짜·성별만 본다)
pub fn rrn_kind(front6: &str, back7: &str, has_sep: bool) -> Option<InfoType> {
    let (f, k) = (front6.as_bytes(), back7.as_bytes());
    if f.len() != 6 || k.is_empty() || !f.iter().all(u8::is_ascii_digit) || !k[0].is_ascii_digit() {
        return None;
    }
    let n = |i: usize| (f[i] - b'0') as usize;
    let g = (k[0] - b'0') as usize;
    // 성별 숫자 → 세기 (0=1800, 1=1900, 2=2000)
    const CENTURY: [usize; 10] = [0, 1, 1, 2, 2, 1, 1, 2, 2, 0];
    let (yymm, dd) = ((n(0) * 10 + n(1)) * 100 + n(2) * 10 + n(3), n(4) * 10 + n(5));
    // 정답표 한 번 조회: 그 달에 있는 날짜 비트 (없는 달·미래 연도는 0)
    if dd >= 32 || (date_table()[CENTURY[g]][yymm] >> dd) & 1 == 0 {
        return None;
    }
    if !has_sep && !rrn_checksum(&format!("{front6}{back7}")) {
        return None;
    }
    Some(if (5..=8).contains(&g) { InfoType::Frn } else { InfoType::Rrn })
}

/// 생년월일 정답표 — `[세기][yy*100 + mm]` = 그 달에 실제로 있는 날짜(1~31)의 비트.
/// 달 00·13~99, 미래 연도(올해 이후), 윤년이 아닌 해의 2월 29일은 비트가 없다.
/// 처음 한 번만 만들고(3 × 10000 × 4바이트 = 120KB) 이후로는 조회 한 번으로 날짜를 판정한다.
fn date_table() -> &'static [[u32; 10_000]; 3] {
    static T: std::sync::OnceLock<Box<[[u32; 10_000]; 3]>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let now = current_year();
        let mut t = Box::new([[0u32; 10_000]; 3]);
        for (c, row) in t.iter_mut().enumerate() {
            for yy in 0..100u32 {
                let year = 1800 + 100 * c as u32 + yy;
                if year > now {
                    continue;
                }
                for mm in 1..=12u32 {
                    let days = days_in_month(year, mm);
                    row[(yy * 100 + mm) as usize] = ((1u64 << (days + 1)) - 2) as u32; // 비트 1..=days
                }
            }
        }
        t
    })
}

// ───────────── 이메일 ─────────────

/// 이메일처럼 보이지만 파일명인 경우 (logo@2x.png 등)
const NON_EMAIL_TLD: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "css", "js", "json", "xml", "html", "htm",
    "txt", "md", "pdf", "zip", "ts", "tsx", "jsx", "py", "rs", "go", "java", "class", "min", "map",
];

pub fn valid_email(s: &str) -> bool {
    let Some((local, domain)) = s.rsplit_once('@') else { return false };
    if local.is_empty() || local.len() > 64 || local.starts_with('.') || local.ends_with('.') || local.contains("..") {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|l| l.is_empty() || l.starts_with('-') || l.ends_with('-')) {
        return false;
    }
    let tld = labels.last().unwrap().to_ascii_lowercase();
    tld.len() >= 2 && tld.bytes().all(|b| b.is_ascii_alphabetic()) && !NON_EMAIL_TLD.contains(&tld.as_str())
}

// ───────────── MAC 주소 ─────────────

pub fn valid_mac(s: &str) -> bool {
    let b = s.as_bytes();
    let sep = b[2];
    if ![5, 8, 11, 14].iter().all(|&i| b[i] == sep) {
        return false;
    }
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>().to_ascii_lowercase();
    hex != "000000000000" && hex != "ffffffffffff"
}

// ───────────── 자격증명 값 ─────────────

/// 실제 값이 아닌 자리표시자/예시 값
pub fn is_placeholder(v: &str) -> bool {
    let l = v.trim().to_ascii_lowercase();
    if l.is_empty() {
        return true;
    }
    const WORDS: &[&str] = &[
        "null", "none", "nil", "true", "false", "undefined", "empty", "password", "passwd", "secret", "token",
        "your_password", "yourpassword", "your-password", "your_token", "your_api_key", "changeme_later", "xxx",
        "xxxx", "string", "required", "optional", "hidden", "masked", "redacted", "n/a", "na", "todo", "tbd",
    ];
    if WORDS.contains(&l.as_str()) {
        return true;
    }
    // 이미 치환된 값 (절반 이상이 *)
    if l.chars().filter(|&c| c == '*').count() * 2 >= l.chars().count() {
        return true;
    }
    // 모두 같은 문자 (****, xxxxxx, ......)
    let first = l.chars().next().unwrap();
    if l.chars().all(|c| c == first) && !first.is_ascii_alphanumeric() || l.chars().all(|c| c == 'x' || c == '*') {
        return true;
    }
    // 템플릿 변수: ${X}, {{x}}, <x>, %s, %(x)s
    (l.starts_with("${") && l.ends_with('}'))
        || (l.starts_with("{{") && l.ends_with("}}"))
        || (l.starts_with('<') && l.ends_with('>'))
        || l == "%s"
        || (l.starts_with("%(") && l.ends_with(")s"))
}

/// 값이 리터럴이 아니라 코드 표현식인지 (os.environ[..], getenv(), $VAR ...)
pub fn is_code_expr(v: &str, quoted: bool) -> bool {
    let l = v.to_ascii_lowercase();
    if l.contains("environ") || l.contains("getenv") || l.contains("process.env") || l.contains("config.") {
        return true;
    }
    if quoted {
        return false;
    }
    l.starts_with('$') || l.starts_with('%') || l.starts_with('{') || l.starts_with('[') || l.contains('(')
        || l.ends_with(')')
        // a.b.c 형태의 속성 접근
        || (l.contains('.') && l.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphabetic() || c == '_')))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 정답표 도입 전의 분기식 구현 (기준)
    fn rrn_kind_reference(front6: &str, back7: &str, has_sep: bool) -> Option<InfoType> {
        let yy: u32 = front6[0..2].parse().ok()?;
        let mm: u32 = front6[2..4].parse().ok()?;
        let dd: u32 = front6[4..6].parse().ok()?;
        let g = back7.as_bytes()[0] - b'0';
        let century = match g {
            1 | 2 | 5 | 6 => 1900,
            3 | 4 | 7 | 8 => 2000,
            9 | 0 => 1800,
            _ => return None,
        };
        let year = century + yy;
        if year > current_year() {
            return None;
        }
        if !(1..=12).contains(&mm) || dd == 0 || dd > days_in_month(year, mm) {
            return None;
        }
        if !has_sep && !rrn_checksum(&format!("{front6}{back7}")) {
            return None;
        }
        Some(if (5..=8).contains(&g) { InfoType::Frn } else { InfoType::Rrn })
    }

    /// 모든 생년월일 6자리(000000~999999) × 성별 숫자 10개 — 기준 구현과 완전히 같은가
    #[test]
    fn rrn_date_table_matches_reference_exhaustive() {
        let mut back = *b"0000000";
        for f in 0..1_000_000u32 {
            let front = format!("{f:06}");
            for g in 0..10u8 {
                back[0] = b'0' + g;
                let b = std::str::from_utf8(&back).unwrap();
                assert_eq!(rrn_kind(&front, b, true), rrn_kind_reference(&front, b, true), "{front}-{b}");
            }
        }
        // 구분자 없는 경우(검증번호) 표본
        let mut x = 7u64;
        for _ in 0..200_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let d = format!("{:013}", x % 10_000_000_000_000);
            assert_eq!(rrn_kind(&d[..6], &d[6..], false), rrn_kind_reference(&d[..6], &d[6..], false), "{d}");
        }
    }

    #[test]
    fn ip_rules() {
        assert!(valid_ipv4("192.168.77.1"));
        assert!(!valid_ipv4("192.168.77.256"));
        assert!(!valid_ipv4("192.168.077.1"));
        assert!(ipv4_is_trivial("255.255.255.0"));
        assert!(!ipv4_is_trivial("10.0.0.1"));
    }

    #[test]
    fn mobile_rules() {
        assert!(valid_mobile("010", "1111", "2222"));
        assert!(!valid_mobile("019", "0111", "2222"));
        assert!(!valid_mobile("010", "111", "2222"));
        assert!(valid_mobile("011", "111", "2222"));
        assert!(!valid_mobile("015", "1111", "2222"));
    }

    #[test]
    fn card_rules() {
        assert!(valid_card("4111111111111111"));
        assert!(valid_card("5500000000000004"));
        assert!(valid_card("378282246310005"));
        assert!(!valid_card("1234567890123456"));
        assert!(!valid_card("4111111111111112"));
    }

    #[test]
    fn rrn_rules() {
        assert_eq!(rrn_kind("881111", "1234567", true), Some(InfoType::Rrn));
        assert_eq!(rrn_kind("881711", "1234567", true), None); // 17월 없음
        assert_eq!(rrn_kind("880230", "1234567", true), None); // 2월 30일 없음
        assert_eq!(rrn_kind("000229", "3234567", true), Some(InfoType::Rrn)); // 2000 윤년
        assert_eq!(rrn_kind("900101", "5234567", true), Some(InfoType::Frn));
        assert_eq!(rrn_kind("990101", "3234567", true), None); // 2099년생 없음
    }

    #[test]
    fn email_rules() {
        assert!(valid_email("hong.gildong@example.co.kr"));
        assert!(!valid_email("logo@2x.png"));
        assert!(!valid_email(".a@b.com"));
    }

    #[test]
    fn mac_rules() {
        assert!(valid_mac("00:1A:2B:3C:4D:5E"));
        assert!(!valid_mac("00:1A-2B:3C:4D:5E"));
        assert!(!valid_mac("00:00:00:00:00:00"));
    }

    #[test]
    fn placeholder_rules() {
        assert!(is_placeholder("********"));
        assert!(is_placeholder("${DB_PASS}"));
        assert!(is_placeholder("<password>"));
        assert!(!is_placeholder("rootPass99!"));
        assert!(is_code_expr("os.environ['X']", false));
        assert!(is_code_expr("input()", false));
        assert!(!is_code_expr("hunter2", false));
    }
}

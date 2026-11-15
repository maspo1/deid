//! OCR 오인식 글자의 맥락 교정.
//!
//! OCR 은 숫자를 비슷한 모양의 글자로 잘못 읽는다 (`0→O/o/이`, `1→l/I/|`, `5→S` …).
//! 무조건 바꾸면 일반 단어가 망가지므로 **맥락**이 있을 때만 교정한다.
//!
//! * 숫자 맥락: 숫자·구분자로 이어진 구간 안에서, 실제 숫자(또는 이미 교정된 글자)와 **붙어 있는** 글자만 교정
//! * 구간 조건: 실제 숫자가 3개 이상 (줄에 "연락처/IP/카드…" 같은 키워드가 있으면 2개 이상)
//! * 교정 결과는 다시 1단계 정규식 + 2단계 사전규칙을 통과해야 검출된다 (오교정이 그대로 오탐이 되지 않음)
//!
//! 글자 수는 바뀌지 않는다(1:1 치환) → 단어 좌표 계산이 그대로 유효하다.

/// 숫자로 오인되기 쉬운 글자 → 숫자
fn as_digit(c: char) -> Option<char> {
    Some(match c {
        'O' | 'o' | 'D' | 'Q' | '이' | 'ㅇ' | '○' | '°' => '0',
        'l' | 'I' | '|' | 'i' | '!' | 'ㅣ' | 'ㄱ' => '1',
        'Z' | 'z' => '2',
        'S' | 's' | '$' => '5',
        'B' => '8',
        'b' | 'G' => '6',
        'g' | 'q' => '9',
        _ => return None,
    })
}

/// 구분자로 오인되기 쉬운 글자 → 구분자
fn as_separator(c: char) -> Option<char> {
    Some(match c {
        '—' | '–' | '~' | '_' | 'ㅡ' => '-',
        '·' | '•' | '．' => '.',
        '：' => ':',
        _ => return None,
    })
}

fn is_sep(c: char) -> bool {
    matches!(c, '.' | '-' | ':' | ',' | '/') || as_separator(c).is_some()
}

/// 줄 전체에 숫자형 정보를 암시하는 키워드가 있는가
pub fn line_has_numeric_context(line: &str) -> bool {
    const KEYS: &[&str] = &[
        "전화", "연락", "핸드폰", "휴대", "폰", "번호", "주민", "외국인", "카드", "계좌", "아이피", "주소", "phone", "tel",
        "mobile", "hp", "ip", "mac", "card", "addr", "no.", "fax", "cell",
    ];
    let l = line.to_lowercase();
    KEYS.iter().any(|k| l.contains(k))
}

/// 단어 하나 교정. 바뀐 것이 없으면 None.
pub fn fix_word(word: &str, numeric_context: bool) -> Option<String> {
    let mut chars: Vec<char> = word.chars().collect();
    let n = chars.len();
    let min_digits = if numeric_context { 2 } else { 3 };
    let mut changed = false;

    // 숫자·오인 글자·구분자로 이루어진 구간 단위로 처리
    let mut i = 0;
    while i < n {
        let in_run = |c: char| c.is_ascii_digit() || as_digit(c).is_some() || is_sep(c);
        if !in_run(chars[i]) {
            i += 1;
            continue;
        }
        let s = i;
        while i < n && in_run(chars[i]) {
            i += 1;
        }
        let run = s..i;
        let real_digits = chars[run.clone()].iter().filter(|c| c.is_ascii_digit()).count();
        if real_digits < min_digits {
            continue;
        }
        // 숫자에 붙은 오인 글자를 양방향으로 반복 교정 (55OO → 5500)
        let mut digitish: Vec<bool> = chars.iter().map(|c| c.is_ascii_digit()).collect();
        let mut progress = true;
        while progress {
            progress = false;
            for k in run.clone() {
                if digitish[k] {
                    continue;
                }
                let Some(d) = as_digit(chars[k]) else { continue };
                let left = k > s && digitish[k - 1];
                let right = k + 1 < i && digitish[k + 1];
                if left || right {
                    chars[k] = d;
                    digitish[k] = true;
                    progress = true;
                    changed = true;
                }
            }
        }
        // 구분자 사이에 홀로 있는 오인 글자 (192.168.0.l → 1) — 이웃 묶음에 숫자가 있을 때만.
        // ".log" 처럼 글자 여러 개로 된 묶음은 숫자 묶음이 아니므로 그대로 둔다.
        for k in run.clone() {
            let Some(d) = as_digit(chars[k]) else { continue };
            if digitish[k] {
                continue;
            }
            let lone = (k == s || is_sep(chars[k - 1])) && (k + 1 == i || is_sep(chars[k + 1]));
            let near = (k >= s + 2 && is_sep(chars[k - 1]) && digitish[k - 2])
                || (k + 2 < i && is_sep(chars[k + 1]) && digitish[k + 2]);
            if lone && near {
                chars[k] = d;
                digitish[k] = true;
                changed = true;
            }
        }
        // 오인 구분자: 양옆이 숫자일 때만
        for k in run.clone() {
            if let Some(sep) = as_separator(chars[k]) {
                if k > s && k + 1 < i && digitish[k - 1] && digitish[k + 1] {
                    chars[k] = sep;
                    changed = true;
                }
            }
        }
        // IP 맥락(점이 2개 이상)에서 숫자 사이 쉼표는 점
        if chars[run.clone()].iter().filter(|&&c| c == '.').count() >= 2 {
            for k in run.clone() {
                if chars[k] == ',' && k > s && k + 1 < i && digitish[k - 1] && digitish[k + 1] {
                    chars[k] = '.';
                    changed = true;
                }
            }
        }
    }
    changed.then(|| chars.into_iter().collect())
}

/// 문자열 안에 올바른 IPv4(a.b.c.d, 각 0~255) 가 있는가 — 그림 속에 IP 가 있다는 맥락
pub fn has_ipv4(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.')).any(|t| {
        let g: Vec<&str> = t.trim_matches('.').split('.').collect();
        g.len() == 4 && g.iter().all(|x| octet(x))
    })
}

fn octet(x: &str) -> bool {
    !x.is_empty() && x.len() <= 3 && !(x.len() > 1 && x.starts_with('0')) && x.parse::<u16>().map_or(false, |v| v <= 255)
}

/// 점이 빠진 IP 추정 — OCR 이 작은 점을 놓친다 (`192.168.4570` → `192.168.45.70`, `198.183921` → `198.18.39.21`).
/// **IP 로는 불가능한 칸(4자리 이상이거나 255 초과)에만** 점을 넣어 정확히 4칸·각 0~255 가 되는 분할을 찾는다.
/// 이미 올바른 칸은 나누지 않고, 날짜(YYYY.MM.DD) 모양은 건드리지 않는다.
/// 분할이 여럿이면 2~3자리 칸이 많은 쪽, 같으면 앞쪽(앞 칸이 짧은 쪽). 2자리 이상 칸이 2개 미만이면 버린다.
/// 추정값이므로 호출 쪽에서 검토필요로 표시한다.
pub fn repair_dropped_dots(word: &str) -> Option<String> {
    // 단어 안의 숫자·점 덩어리 (앞뒤 점 제외, 점 1개 이상)
    let b = word.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let s = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
            i += 1;
        }
        let mut e = i;
        while e > s && b[e - 1] == b'.' {
            e -= 1;
        }
        let core = &word[s..e];
        if let Some(fixed) = split_octets(core) {
            return Some(format!("{}{fixed}{}", &word[..s], &word[e..]));
        }
    }
    None
}

fn split_octets(core: &str) -> Option<String> {
    if !core.contains('.') || core.contains("..") {
        return None;
    }
    let groups: Vec<&str> = core.split('.').collect();
    let digits: usize = groups.iter().map(|g| g.len()).sum();
    if groups.len() >= 4 || !(4..=12).contains(&digits) || groups.iter().all(|g| octet(g)) {
        return None;
    }
    // 날짜 모양 (2026.11.14, 1999.1.2, 점이 빠진 2024.1026, 일부만 읽힌 2024.01) — 연도 뒤 숫자가 월·일 4자리 이하
    if groups[0].len() == 4 && (groups[0].starts_with("19") || groups[0].starts_with("20")) && digits - 4 <= 4 {
        return None;
    }
    // 칸마다 가능한 나눔 (올바른 칸은 그대로 1조각)
    fn parts(g: &str) -> Vec<Vec<String>> {
        if octet(g) {
            return vec![vec![g.to_string()]];
        }
        let mut out = Vec::new();
        fn rec(rest: &str, acc: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
            if rest.is_empty() {
                if acc.len() >= 2 {
                    out.push(acc.clone());
                }
                return;
            }
            for l in 1..=rest.len().min(3) {
                if octet(&rest[..l]) {
                    acc.push(rest[..l].to_string());
                    rec(&rest[l..], acc, out);
                    acc.pop();
                }
            }
        }
        rec(g, &mut Vec::new(), &mut out);
        out
    }
    let options: Vec<Vec<Vec<String>>> = groups.iter().map(|g| parts(g)).collect();
    let mut best: Option<(usize, Vec<String>)> = None;
    fn walk(i: usize, options: &[Vec<Vec<String>>], acc: &mut Vec<String>, best: &mut Option<(usize, Vec<String>)>) {
        if i == options.len() {
            if acc.len() == 4 {
                let score = acc.iter().filter(|x| x.len() >= 2).count();
                if best.as_ref().map_or(true, |b| score > b.0) {
                    *best = Some((score, acc.clone()));
                }
            }
            return;
        }
        for p in &options[i] {
            if acc.len() + p.len() > 4 {
                continue;
            }
            acc.extend(p.iter().cloned());
            walk(i + 1, options, acc, best);
            acc.truncate(acc.len() - p.len());
        }
    }
    walk(0, &options, &mut Vec::new(), &mut best);
    // 한 자리 칸만 잔뜩 생기는 무리한 분할(300.1 → 3.0.0.1)은 버림
    best.filter(|(score, _)| *score >= 2).map(|(_, v)| v.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_dots_repaired_only_when_impossible_as_ip() {
        // OCR 이 점을 놓친 판독 → 정답
        for (ocr, want) in [
            ("192.168.4570", "192.168.45.70"),
            ("192.168.4575", "192.168.45.75"),
            ("192.16841.92", "192.168.41.92"),
            ("198.1832.47", "198.18.32.47"),
            ("198.183921", "198.18.39.21"),
            ("335.20.30", "3.35.20.30"),
            ("由口198.1832.47", "由口198.18.32.47"),
            ("198.183247", "198.18.32.47"),
        ] {
            assert_eq!(repair_dropped_dots(ocr).as_deref(), Some(want), "{ocr}");
        }
        for keep in ["192.168.45.70", "1.2.3", "2026.11.14", "2024.1026", "2024.01", "2024.105", "3.14", "12345", "v1.2.3", "300.1", "1999.12.31", "010-1234-5678"] {
            assert_eq!(repair_dropped_dots(keep), None, "{keep}");
        }
        assert!(has_ipv4("由口192.168.1.99 [x]") && !has_ipv4("v1.2.3 2026.11.14"));
    }

    #[test]
    fn fixes_numeric_context() {
        assert_eq!(fix_word("10.20.30.4이", false).as_deref(), Some("10.20.30.40"));
        assert_eq!(fix_word("O1O-12OO-5678", false).as_deref(), Some("010-1200-5678"));
        assert_eq!(fix_word("192,168.0.l", false).as_deref(), Some("192.168.0.1"));
        assert_eq!(fix_word("010—1234—5678", false).as_deref(), Some("010-1234-5678"));
    }

    #[test]
    fn keeps_words() {
        assert_eq!(fix_word("Bob", false), None);
        assert_eq!(fix_word("S3cure", false), None); // 숫자 1개 → 맥락 부족
        assert_eq!(fix_word("lOl", false), None);
        assert_eq!(fix_word("010-1200-5678.log", false), None); // 확장자 log 는 숫자와 붙어 있지 않음
        assert_eq!(fix_word("2026-11-12", false), None);
    }

    #[test]
    fn keyword_context_lowers_threshold() {
        // 실제 숫자 2개: 키워드 맥락이 있을 때만 교정
        assert_eq!(fix_word("1O5", false), None);
        assert_eq!(fix_word("1O5", true).as_deref(), Some("105"));
        assert!(line_has_numeric_context("연락처 010-..."));
        assert!(!line_has_numeric_context("바탕 화면의 검색 결과"));
    }
}

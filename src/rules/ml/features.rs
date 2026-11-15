//! 변형된 숫자 정보 후보 추출 + 맥락 특징 벡터.
//!
//! 후보: 숫자 묶음이 **임의의 기호**(1~4자, 영문·숫자·줄바꿈 제외)로 이어진 덩어리.
//! `192-168-0-1`, `0109999-1234`, `010_9999_1234`, `(010)9999-1234`, `4111.1111.1111.1111` …
//! 덩어리 안의 연속된 묶음(창, window) 단위로 특징을 만든다 — 학습과 추론이 같은 함수를 쓴다.
//!
//! 특징은 **구분자 종류와 무관한 모양**(묶음 수, 길이, 사전 규칙 통과 여부)과
//! **앞뒤 맥락**(키워드, 대입 기호, 주변 숫자 밀도)으로 구성된다.

use crate::common::{char_after, char_before, context_window};
use crate::rules::dict;
use regex::Regex;
use std::sync::OnceLock;

/// 후보 하나만 보고 만드는 특징 수
pub const N_BASE: usize = 55;
/// + 직관·사전·문서 단위 특징 + 레코드 구조 특징 + 구분자 규칙·표 특징
pub const N_FEATURES: usize = N_BASE + 18 + 6 + 4;

/// 분류 클래스 (소프트맥스 출력 순서)
pub const CLASSES: [&str; 5] = ["none", "phone", "ip", "card", "rrn"];
pub const C_NONE: usize = 0;
pub const C_PHONE: usize = 1;
pub const C_IP: usize = 2;
pub const C_CARD: usize = 3;
pub const C_RRN: usize = 4;

/// 후보 창: 원문 위치 + 숫자 묶음 + 구분자
#[derive(Clone, Debug)]
pub struct Candidate {
    pub start: usize,
    pub end: usize,
    pub groups: Vec<String>,
    pub seps: Vec<String>,
    /// 덩어리에서 창 바로 앞/뒤 구분자 (창이 덩어리 끝이면 None)
    pub sep_before: Option<String>,
    pub sep_after: Option<String>,
    /// 같은 덩어리에서 나온 창은 같은 번호
    pub run_id: usize,
    /// 덩어리 안에서 이 창이 복원 가능한 레코드인가
    pub rec: Record,
}

/// 숫자 덩어리 안의 레코드 구조.
///
/// 변형 유출은 프로그램으로 하므로 정형화된 규칙을 따른다: 레코드를 구분자(`,` `;` `|` 공백 `[]` …)로 잇고,
/// 레코드 안은 하나의 규칙(같은 기호 / 자리별 고정 기호 / 작은 기호 집합)으로 바꾼다.
/// * `88.11.25.1,14.20.1.3`, `88^11^25*1,88&11$25*1`, `[88^11^25*1][88&11$25*1]` → 레코드로 복원 가능
/// * `21.0.88.11.25.1.7`, `3^4*55-12^9*200-1` → 어디서 끊어도 하나의 규칙으로 복원할 수 없음
#[derive(Clone, Debug, Default)]
pub struct Record {
    /// 덩어리 안에 묻힌 창 (앞이나 뒤에 다른 묶음이 이어짐)
    pub embedded: bool,
    /// 묻혔어도 양 끝이 레코드 구분자라 하나의 레코드로 떼어낼 수 있음 (묻히지 않았으면 true)
    pub recoverable: bool,
    /// 같은 구분자로 나눈 레코드 수
    pub records: usize,
    /// 그중 IP 모양(4묶음, 각 ≤255) 레코드 비율
    pub ip_share: f32,
    /// IP 모양 레코드들의 자리별 기호가 모두 같음
    pub same_pattern: bool,
    /// 레코드 안 기호가 모두 한 글자이고, 레코드 전체에서 쓰인 기호 종류가 4개 이하 (프로그램 변형 흔적)
    pub small_alphabet: bool,
}

/// 레코드 구분자로 쓰이는 기호인가 (공백이 섞인 기호 `: `, ` -> ` 도 필드 구분으로 본다)
fn record_delim(t: &str) -> bool {
    !t.is_empty() && (t.chars().any(char::is_whitespace) || t.chars().all(|ch| ",;|/[](){}<>，、".contains(ch)))
}

fn ip_shaped(groups: &[&str]) -> bool {
    groups.len() == 4 && groups.iter().all(|g| g.len() <= 3 && g.parse::<u32>().map_or(false, |v| v <= 255))
}

/// 창 `a..=z` 의 레코드 구조
fn record_info(groups: &[&str], seps: &[&str], a: usize, z: usize) -> Record {
    let n = groups.len();
    let inner = &seps[a..z];
    let left = (a > 0).then(|| seps[a - 1]);
    // 끝의 포트(:8080)·대역(/24)은 레코드 경계가 아니다
    let tail = z + 2 == n && groups[n - 1].len() <= 5 && matches!(seps[z], ":" | "/");
    let right = (z + 1 < n && !tail).then(|| seps[z]);
    let embedded = left.is_some() || right.is_some();
    let delim = match (left, right) {
        (None, None) => None,
        (Some(l), Some(r)) if l != r => return Record { embedded, ..Default::default() },
        (l, r) => l.or(r),
    };
    let Some(d) = delim else {
        return Record { embedded: false, recoverable: true, records: 1, ..Default::default() };
    };
    if !record_delim(d) || inner.contains(&d) {
        return Record { embedded, ..Default::default() };
    }
    // 덩어리 전체를 같은 구분자로 나눈 레코드들
    let mut recs: Vec<(usize, usize)> = Vec::new();
    let mut st = 0;
    for (i, s) in seps.iter().enumerate() {
        if *s == d {
            recs.push((st, i));
            st = i + 1;
        }
    }
    recs.push((st, n - 1));
    let ip_recs: Vec<&(usize, usize)> = recs.iter().filter(|(x, y)| ip_shaped(&groups[*x..=*y])).collect();
    let pattern = |(x, y): &(usize, usize)| &seps[*x..*y];
    let same_pattern = ip_recs.len() >= 2 && ip_recs.iter().all(|r| pattern(r) == pattern(ip_recs[0]));
    let mut alphabet: Vec<&str> = ip_recs.iter().flat_map(|r| pattern(r).iter().copied()).collect();
    let single = alphabet.iter().all(|s| s.chars().count() == 1);
    alphabet.sort();
    alphabet.dedup();
    Record {
        embedded,
        recoverable: true,
        records: recs.len(),
        ip_share: ip_recs.len() as f32 / recs.len() as f32,
        same_pattern,
        small_alphabet: !ip_recs.is_empty() && single && alphabet.len() <= 4,
    }
}

impl Candidate {
    pub fn digits(&self) -> String {
        self.groups.concat()
    }
}

fn run_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"[0-9]+(?:[^\p{L}\p{N}\r\n]{1,4}[0-9]+)*").unwrap())
}

/// 숫자 개수·묶음 모양이 어떤 클래스든 될 수 있는 창인가 (계산량을 줄이는 사전 거르기)
fn plausible(groups: &[&str]) -> bool {
    let d: usize = groups.iter().map(|g| g.len()).sum();
    let first = groups[0].as_bytes();
    let phone = (d == 10 || d == 11) && first.starts_with(b"01");
    let ip = groups.len() == 4 && groups.iter().all(|g| g.len() <= 3);
    let card = (13..=19).contains(&d) && groups.len() <= 5 && (b'2'..=b'6').contains(&first[0]);
    let rrn = d == 13 && groups.len() <= 3;
    phone || ip || card || rrn
}

/// 텍스트에서 후보 창 목록 (묶음 2~6개 연속 구간)
pub fn candidates(text: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    // 덩어리마다 새로 할당하지 않고 재사용 (숫자 하나짜리 덩어리가 대부분이라 할당이 시간 대부분을 먹었다)
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (run_id, m) in run_regex().find_iter(text).enumerate() {
        let s = m.as_str();
        let base = m.start();
        // 묶음이 하나뿐(구분자 없음)이면 후보가 아님
        if s.bytes().all(|c| c.is_ascii_digit()) {
            continue;
        }
        // 묶음 위치
        spans.clear();
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i].is_ascii_digit() {
                let st = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                spans.push((st, i));
            } else {
                i += 1;
            }
        }
        if spans.len() < 2 {
            continue;
        }
        let groups: Vec<&str> = spans.iter().map(|&(a, z)| &s[a..z]).collect();
        let seps: Vec<&str> = spans.windows(2).map(|w| &s[w[0].1..w[1].0]).collect();
        let n = spans.len();
        for a in 0..n {
            for z in a + 1..n.min(a + 6) {
                // 문자열 복사는 그럴듯한 창에만
                if !plausible(&groups[a..=z]) {
                    continue;
                }
                out.push(Candidate {
                    start: base + spans[a].0,
                    end: base + spans[z].1,
                    groups: groups[a..=z].iter().map(|x| x.to_string()).collect(),
                    seps: seps[a..z].iter().map(|x| x.to_string()).collect(),
                    sep_before: (a > 0).then(|| seps[a - 1].to_string()),
                    sep_after: (z + 1 < n).then(|| seps[z].to_string()),
                    run_id,
                    rec: record_info(&groups, &seps, a, z),
                });
            }
        }
    }
    out
}

static KW_PHONE: Kw = Kw::new(&[
    "hp", "phone", "tel", "mobile", "cell", "contact", "call", "연락", "전화", "휴대", "핸드폰", "폰", "번호", "문의", "담당",
]);
static KW_IP: Kw = Kw::new(&[
    "ip", "host", "server", "addr", "gateway", "gw", "dns", "ping", "proxy", "src", "dst", "remote", "allow", "deny", "route",
    "서버", "접속", "주소", "아이피", "호스트", "게이트웨이", "->", "from", "inet", "listen", "bind",
]);
static KW_CARD: Kw = Kw::new(&["card", "카드", "결제", "visa", "master", "amex", "pan", "cc=", "cc:", "신용"]);
static KW_RRN: Kw = Kw::new(&["주민", "rrn", "ssn", "jumin", "외국인", "생년월일", "등록번호", "resident"]);
static KW_DATE: Kw = Kw::new(&[
    "date", "time", "날짜", "일자", "일시", "기간", "작성일", "created", "updated", "modified", "timestamp", "년", "월", "일 ",
    "expire", "until", "since", "일정", "schedule", "deadline", "마감", "까지", "부터", "주차", "기한", "예정", "회의",
]);
static KW_VERSION: Kw = Kw::new(&["version", "ver", "build", "rev", "버전", "빌드", "release", "patch"]);
static KW_MONEY: Kw = Kw::new(&["원", "$", "₩", "price", "금액", "가격", "amount", "krw", "usd", "total", "합계", "cost", "잔액"]);
static KW_ID: Kw = Kw::new(&[
    "account", "계좌", "주문", "order", "serial", "일련", "invoice", "송장", "isbn", "사업자", "품번", "모델", "model", "sku",
    "rgb", "color", "mask", "size", "좌표", "coord", "lat", "lng",
]);

/// 키워드 목록 — 처음 쓸 때 아호-코라식 오토마타를 한 번 만들어 두고, 문자열을 **한 번만** 훑어 모든 키워드를 찾는다.
struct Kw {
    words: &'static [&'static str],
    ac: std::sync::OnceLock<aho_corasick::AhoCorasick>,
}

impl Kw {
    const fn new(words: &'static [&'static str]) -> Kw {
        Kw { words, ac: std::sync::OnceLock::new() }
    }

    fn ac(&self) -> &aho_corasick::AhoCorasick {
        self.ac.get_or_init(|| aho_corasick::AhoCorasick::new(self.words).expect("키워드 오토마타"))
    }
}

/// 짧은 영문 키워드(4자 이하)는 **단어 단위**로만 인정 —
/// 부분 문자열로 비교하면 "server" 안의 `ver`, "php" 안의 `hp`, "zip" 안의 `ip` 가 잘못 걸린다.
fn whole_word_only(k: &str) -> bool {
    k.len() <= 4 && k.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// 키워드 포함 여부 (겹친 일치까지 모두 보므로, 단어 경계에 걸려 버려진 일치 뒤의 진짜 일치도 놓치지 않는다)
fn has_any(s: &str, kws: &Kw) -> bool {
    let b = s.as_bytes();
    kws.ac().find_overlapping_iter(s).any(|m| {
        if !whole_word_only(kws.words[m.pattern().as_usize()]) {
            return true;
        }
        let left = m.start() == 0 || !b[m.start() - 1].is_ascii_alphanumeric();
        let right = m.end() == b.len() || !b[m.end()].is_ascii_alphanumeric();
        left && right
    })
}

/// 표의 칸 안에 있는가 (`<td>…</td>`, `| … |`, 탭으로 나눈 줄)
fn table_cell(text: &str, start: usize, end: usize) -> bool {
    let ls = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let le = text[end..].find('\n').map_or(text.len(), |i| end + i);
    let (before, after) = (text[ls..start].trim_end(), text[end..le].trim_start());
    let lower = before.to_ascii_lowercase();
    before.ends_with('|')
        || after.starts_with('|')
        || lower.ends_with("<td>")
        || lower.ends_with("<th>")
        || after.to_ascii_lowercase().starts_with("</td")
        || before.contains('\t')
        || after.contains('\t')
}

/// 앞 맥락에 IP 단서가 있는가 (모델 앞단 표 검사용)
pub(crate) fn ip_context(text: &str, start: usize) -> bool {
    has_any(&context_window(text, start, start, 40, 0), &KW_IP)
}

fn digit_density(s: &str) -> f32 {
    let n = s.chars().count().max(1);
    s.chars().filter(|c| c.is_ascii_digit()).count() as f32 / n as f32
}

/// 월.일 쌍으로 읽히는가 (10.26 / 10.26.~10.31 / 10/26-10/31)
pub(crate) fn month_day_pairs(groups: &[String]) -> bool {
    let v: Vec<u32> = groups.iter().map(|g| g.parse().unwrap_or(99)).collect();
    let pair = |m: u32, d: u32| (1..=12).contains(&m) && (1..=31).contains(&d);
    match v.len() {
        2 => pair(v[0], v[1]),
        4 => pair(v[0], v[1]) && pair(v[2], v[3]),
        _ => false,
    }
}

fn is_date_like(groups: &[String]) -> bool {
    let p = |s: &str| s.parse::<u32>().unwrap_or(99_999);
    if groups.len() >= 3 && groups[0].len() == 4 {
        let y = p(&groups[0]);
        return (1900..=2099).contains(&y) && (1..=12).contains(&p(&groups[1])) && (1..=31).contains(&p(&groups[2]));
    }
    if groups.len() >= 3 && groups[2].len() == 4 {
        let y = p(&groups[2]);
        return (1900..=2099).contains(&y) && p(&groups[0]) <= 31 && p(&groups[1]) <= 31;
    }
    let g = &groups[0];
    if g.len() == 8 {
        let (y, m, d) = (p(&g[..4]), p(&g[4..6]), p(&g[6..]));
        return (1900..=2099).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d);
    }
    false
}

/// 숫자열 → (국번, 가운데, 끝) 으로 휴대폰 해석
pub fn mobile_parts(d: &str) -> Option<(&str, &str, &str)> {
    match d.len() {
        11 => Some((&d[..3], &d[3..7], &d[7..])),
        10 => Some((&d[..3], &d[3..6], &d[6..])),
        _ => None,
    }
}

pub fn ip_string(c: &Candidate) -> Option<String> {
    (c.groups.len() == 4).then(|| c.groups.join("."))
}

/// 특징 벡터
pub fn features(text: &str, c: &Candidate) -> [f32; N_BASE] {
    let mut f = [0f32; N_BASE];
    let g = &c.groups;
    let d = c.digits();
    let lens: Vec<usize> = g.iter().map(String::len).collect();
    let b = |x: bool| if x { 1.0 } else { 0.0 };

    f[0] = g.len() as f32;
    for i in 0..6 {
        f[1 + i] = *lens.get(i).unwrap_or(&0) as f32;
    }
    f[7] = d.len() as f32;
    f[8] = *lens.iter().max().unwrap() as f32;
    f[9] = *lens.iter().min().unwrap() as f32;
    let mut uniq: Vec<&String> = c.seps.iter().collect();
    uniq.sort();
    uniq.dedup();
    f[10] = uniq.len() as f32;
    let sep_all: String = c.seps.concat();
    f[11] = b(sep_all.contains('-'));
    f[12] = b(sep_all.contains('.'));
    f[13] = b(sep_all.contains(' ') || sep_all.contains('\t'));
    f[14] = b(sep_all.contains('_'));
    f[15] = b(sep_all.contains('/') || sep_all.contains('\\'));
    f[16] = b(sep_all.contains(':'));
    f[17] = b(sep_all.contains(','));
    f[18] = b(sep_all.chars().any(|ch| !matches!(ch, '-' | '.' | ' ' | '\t' | '_' | '/' | '\\' | ':' | ',')));
    f[19] = b(uniq.len() == 1);
    f[20] = b(d.starts_with("01"));
    f[21] = b(d.starts_with("010"));
    f[22] = b(mobile_parts(&d).map_or(false, |(a, m, l)| dict::valid_mobile(a, m, l)));
    let ip = ip_string(c);
    f[23] = b(ip.as_deref().map_or(false, |s| dict::valid_ipv4(s) && !dict::ipv4_is_trivial(s)));
    f[24] = b(f[23] > 0.0 && {
        let o: Vec<u32> = g.iter().map(|x| x.parse().unwrap_or(999)).collect();
        o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168) || (o[0] == 169 && o[1] == 254)
    });
    f[25] = b(dict::valid_card(&d));
    f[26] = b((13..=19).contains(&d.len()) && dict::valid_card_iin(&d));
    let rrn = d.len() == 13 && dict::rrn_kind(&d[..6], &d[6..], true).is_some();
    f[27] = b(rrn);
    f[28] = b(d.len() == 13 && dict::rrn_kind(&d[..6], &d[6..], false).is_some());
    f[29] = b(is_date_like(g));
    f[30] = b(sep_all.contains(':') && g.iter().all(|x| x.len() <= 2 && x.parse::<u32>().unwrap_or(99) <= 59));
    let alpha = |ch: Option<char>| ch.map_or(false, |x| x.is_ascii_alphabetic());
    f[31] = b(alpha(char_before(text, c.start)));
    f[32] = b(alpha(char_after(text, c.end)));
    f[33] = b(c.sep_before.is_some());
    f[34] = b(c.sep_after.is_some());
    let same = |n: &Option<String>| n.as_ref().map_or(false, |s| c.seps.contains(s));
    f[35] = b(same(&c.sep_before) || same(&c.sep_after));

    let before = context_window(text, c.start, c.start, 40, 0);
    let after = context_window(text, c.end, c.end, 0, 16);
    f[36] = b(has_any(&before, &KW_PHONE));
    f[37] = b(has_any(&before, &KW_IP));
    f[38] = b(has_any(&before, &KW_CARD));
    f[39] = b(has_any(&before, &KW_RRN));
    f[40] = b(has_any(&before, &KW_DATE) || has_any(&after, &KW_DATE));
    f[41] = b(has_any(&before, &KW_VERSION));
    f[42] = b(has_any(&before, &KW_MONEY) || has_any(&after, &KW_MONEY));
    f[43] = b(has_any(&before, &KW_ID));
    let tb = before.trim_end_matches([' ', '"', '\'', '\t', '(', '[']);
    f[44] = b(tb.ends_with('=') || tb.ends_with(':'));
    f[45] = b(before.ends_with('"') || before.ends_with('\''));
    f[46] = (digit_density(&before) + digit_density(&after)) / 2.0;
    f[47] = b(sep_all.contains('(') || sep_all.contains(')') || before.ends_with('('));
    f[48] = g.iter().skip(1).filter(|x| x.len() > 1 && x.starts_with('0')).count() as f32;
    f[49] = b(sep_all.chars().any(char::is_whitespace) && sep_all.chars().any(|ch| !ch.is_whitespace()));
    f[50] = b(has_any(&after, &KW_PHONE) || after.starts_with("로") || after.starts_with("으로"));
    f[51] = b(has_any(&after, &KW_IP) || after.starts_with(":") || after.starts_with("/"));
    // 날짜 범위: `~` 는 범위 기호로 흔히 쓰인다 (10.26.~10.31.)
    f[52] = b(sep_all.contains('~') || sep_all.contains('～') || sep_all.contains('〜'));
    f[53] = b(month_day_pairs(g));
    let range_mark = |s: &str| s.contains('~') || s.contains('～');
    f[54] = b(c.sep_before.as_deref().map_or(false, range_mark)
        || c.sep_after.as_deref().map_or(false, range_mark)
        || before.trim_end().ends_with('~')
        || after.trim_start_matches('.').trim_start().starts_with('~'));
    f
}

/// IP 모양(4묶음, 각 ≤255)이면 옥텟
pub fn quad(c: &Candidate) -> Option<[u32; 4]> {
    if c.groups.len() != 4 || c.groups.iter().any(|g| g.len() > 3) {
        return None;
    }
    let v: Vec<u32> = c.groups.iter().map(|g| g.parse().unwrap_or(999)).collect();
    v.iter().all(|&x| x <= 255).then(|| [v[0], v[1], v[2], v[3]])
}

/// 구분자 서명 (같은 변환 규칙인지 비교용)
fn signature(c: &Candidate) -> Vec<&str> {
    let mut s: Vec<&str> = c.seps.iter().map(String::as_str).collect();
    s.sort();
    s.dedup();
    s
}

/// 문서 전체를 보고 후보별 특징 벡터 생성 (학습·추론 공용).
///
/// 후보 특징(N_BASE) + 직관·사전·문서 단위 특징:
/// * 자연스러움 확률(1단 모델), 연속 숫자·등차·반복, 사설/흔한 사설/비호스트/잘 알려진/국내 대역
/// * **기계적 치환 흔적**: 같은 구분자 규칙으로 바뀐 다른 IP·전화번호가 문서에 몇 개 있는가
/// * **다의성**: 날짜·월일·시각·작은 버전 번호·인공 배열로도 읽히는가 (해석이 여럿이면 복원 가능성이 낮다)
pub fn doc_features(text: &str, nat: &super::gbdt::Model) -> Vec<(Candidate, Vec<f32>)> {
    doc_features_from(text, candidates(text), nat)
}

/// 미리 만든 후보로 특징 생성 (후보가 없으면 문서 전체 소문자 변환 등도 하지 않음)
pub fn doc_features_from(text: &str, cands: Vec<Candidate>, nat: &super::gbdt::Model) -> Vec<(Candidate, Vec<f32>)> {
    doc_features_keep(text, cands, nat, |_| true)
}

/// 문서 단위 특징(같은 규칙으로 바뀐 다른 번호 수 등)은 **모든** 후보로 세고, 후보별 특징은 `keep` 인 후보만 만든다.
pub fn doc_features_keep(
    text: &str,
    cands: Vec<Candidate>,
    nat: &super::gbdt::Model,
    keep: impl Fn(&Candidate) -> bool,
) -> Vec<(Candidate, Vec<f32>)> {
    use super::intuition::natural_features;
    if cands.is_empty() {
        return Vec::new();
    }
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    // 후보별 1단 결과
    let pnat: Vec<Option<f32>> = cands.iter().map(|c| quad(c).map(|o| nat.predict_proba(&natural_features(&o))[1])).collect();
    // 서명별로 "자연스러운 IP / 전화 / 인공 배열" 이 나온 덩어리 집합
    use std::collections::{HashMap, HashSet};
    let mut ip_runs: HashMap<Vec<&str>, HashSet<usize>> = HashMap::new();
    let mut ph_runs: HashMap<Vec<&str>, HashSet<usize>> = HashMap::new();
    let mut art_runs: HashMap<Vec<&str>, HashSet<usize>> = HashMap::new();
    let mut dotted = HashSet::new();
    for (c, p) in cands.iter().zip(&pnat) {
        let sig = signature(c);
        if let (Some(o), Some(p)) = (quad(c), p) {
            let ip = format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3]);
            if dict::valid_ipv4(&ip) {
                if c.seps.iter().all(|s| s == ".") {
                    dotted.insert(c.run_id);
                }
                if *p >= 0.5 {
                    ip_runs.entry(sig.clone()).or_default().insert(c.run_id);
                } else if *p < 0.3 {
                    art_runs.entry(sig.clone()).or_default().insert(c.run_id);
                }
            }
        }
        let d = c.digits();
        if mobile_parts(&d).map_or(false, |(a, m, l)| dict::valid_mobile(a, m, l)) {
            ph_runs.entry(sig).or_default().insert(c.run_id);
        }
    }
    let others = |m: &HashMap<Vec<&str>, HashSet<usize>>, sig: &Vec<&str>, run: usize| -> f32 {
        m.get(sig).map_or(0, |s| s.len() - s.contains(&run) as usize).min(5) as f32
    };
    let lower = text.to_lowercase();
    let doc_ip_kw = b(has_any(&lower, &KW_IP));
    let doc_phone_kw = b(has_any(&lower, &KW_PHONE));

    cands
        .iter()
        .zip(&pnat)
        .filter(|(c, _)| keep(c))
        .map(|(c, p)| {
            let mut v = features(text, c).to_vec();
            let sig = signature(c);
            let q = quad(c);
            let nf = q.map(|o| natural_features(&o));
            let inf = q.map(|o| super::ipdict::info(&o));
            let g = &c.groups;
            v.push(b(q.is_some()));
            v.push(p.unwrap_or(0.0));
            v.push(nf.map_or(0.0, |f| f[12]));
            v.push(nf.map_or(0.0, |f| f[13]));
            v.push(nf.map_or(0.0, |f| b(f[15] > 0.0 || f[16] >= 0.75)));
            v.push(b(inf.map_or(false, |i| i.private)));
            v.push(b(inf.map_or(false, |i| i.common_private)));
            v.push(b(inf.map_or(false, |i| i.non_host)));
            v.push(b(inf.map_or(false, |i| i.well_known)));
            v.push(b(inf.map_or(false, |i| i.kr)));
            // 복원 가능성: 모든 구분자가 같은 한 글자(점 아님) → 기계적 일괄 치환으로 보임
            v.push(b(sig.len() == 1 && sig[0].chars().count() == 1 && sig[0] != "."));
            v.push(others(&ip_runs, &sig, c.run_id));
            v.push(others(&ph_runs, &sig, c.run_id));
            v.push(others(&art_runs, &sig, c.run_id));
            v.push(dotted.len().min(5) as f32);
            // 다의성: 다른 해석의 개수
            let small = g.iter().all(|x| x.parse::<u32>().unwrap_or(999) < 20);
            let ambiguity = [
                is_date_like(g),
                month_day_pairs(g),
                c.seps.iter().any(|s| s.contains(':')) && g.iter().all(|x| x.len() <= 2),
                small && q.is_some(),
                p.map_or(false, |p| p < 0.3),
            ]
            .iter()
            .filter(|&&x| x)
            .count();
            v.push(ambiguity as f32);
            v.push(doc_ip_kw);
            v.push(doc_phone_kw);
            // 레코드 구조 (복원 가능성)
            let r = &c.rec;
            v.push(b(r.embedded && r.recoverable));
            v.push(r.records.min(6) as f32);
            v.push(r.ip_share);
            v.push(b(r.same_pattern));
            v.push(b(r.small_alphabet));
            let wrap = |o: char, cl: char| char_before(text, c.start) == Some(o) && char_after(text, c.end) == Some(cl);
            v.push(b(wrap('[', ']') || wrap('(', ')') || wrap('{', '}') || wrap('<', '>')));
            // 구분자 규칙 (아스키 코드 표, 모든 클래스 공통): 한 규칙으로 바꿨는가 / 그 기호가 숫자표 열 구분자인가
            let rule = dict::uniform_sep(&c.seps);
            v.push(b(rule.is_some()));
            v.push(b(rule.map_or(false, |(_, k)| k == dict::SEP_TABLE)));
            // 오름차순 숫자 열 (쪽·번호 목록: 21,23,53,60 / 42, 58, 60, 64)
            let nums: Vec<u64> = g.iter().map(|x| x.parse().unwrap_or(u64::MAX)).collect();
            v.push(b(nums.len() >= 3 && nums.windows(2).all(|w| w[0] < w[1])));
            v.push(b(table_cell(text, c.start, c.end)));
            debug_assert_eq!(v.len(), N_FEATURES);
            (c.clone(), v)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_obfuscated() {
        let t = "hp = \"0109999-1234\", ip 192-168-0-1";
        let c = candidates(t);
        assert!(c.iter().any(|c| &t[c.start..c.end] == "0109999-1234"));
        assert!(c.iter().any(|c| &t[c.start..c.end] == "192-168-0-1"));
    }
}

#[cfg(test)]
mod kw_tests {
    use super::*;

    /// 아호-코라식 이전 구현 (기준)
    fn has_any_reference(s: &str, kws: &[&str]) -> bool {
        let words: Vec<&str> = s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
        kws.iter().any(|k| if whole_word_only(k) { words.iter().any(|w| w == k) } else { s.contains(k) })
    }

    #[test]
    fn aho_corasick_matches_reference() {
        let lists = [&KW_PHONE, &KW_IP, &KW_CARD, &KW_RRN, &KW_DATE, &KW_VERSION, &KW_MONEY, &KW_ID];
        let bits = [
            "server", "ver", "php", "hp", "zip", "ip", " ", "-", ":", "=", "\n", "서버", "전화", "phone", "hp:", "(ip)", "ipv4", "ver2",
            "release", "x", "1", "일정", "원", "₩", "cc=", "cc", "→", "a", "->", "host:", "카드번호", "ssn", "jumin", "년 ",
        ];
        // 결정적 의사 난수로 조각을 이어 붙인 문자열 2만 개
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut checked = 0;
        for _ in 0..20_000 {
            let mut s = String::new();
            for _ in 0..(seed % 7 + 1) {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                s.push_str(bits[(seed >> 33) as usize % bits.len()]);
            }
            for l in lists {
                assert_eq!(has_any(&s, l), has_any_reference(&s, l.words), "{s:?}");
                checked += 1;
            }
        }
        assert_eq!(checked, 160_000);
        // 오탐 #23 회귀: 단어 안의 짧은 키워드는 무시
        assert!(!has_any("server 211-234-10-5", &KW_VERSION));
        assert!(has_any("server 211-234-10-5", &KW_IP));
        assert!(!has_any("php zip", &KW_IP) && !has_any("php", &KW_PHONE));
    }
}

//! 학습용 합성 데이터 생성기 (exe 에는 쓰이지 않음 — `cargo run --release --example train_ml`).
//!
//! 정보 유출자가 정규식을 피하려고 쓰는 변형을 무작위로 만든다.
//! * 구분자 치환: `-` → `.`, `_`, `/`, `:`, `,`, `|`, `~`, `*`, `#`, 공백, `--`, ` - ` …
//! * 구분자 누락/혼합: `0109999-1234`, `010.9999_1234`
//! * 괄호: `(010)9999-1234`
//! 함정(정답 없음): 날짜, 시각, 버전, 금액, 표 숫자, 계좌·사업자·주문번호, ISBN, 일반전화, 좌표, 잘못된 번호 …
//!
//! 각 표본은 실제 탐지와 같은 `features::candidates` 로 후보를 뽑아 특징을 만든다.
//! 정답 위치의 후보 = 정답 클래스, 같은 줄의 다른 후보 = none.

use super::features::{self, C_CARD, C_IP, C_NONE, C_PHONE, C_RRN};
use super::{gbdt, intuition, ipdict};

/// 결정적 난수 (xorshift64*)
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn range(&mut self, a: u32, b: u32) -> u32 {
        a + (self.next() % (b - a + 1) as u64) as u32
    }
    pub fn chance(&mut self, p: f32) -> bool {
        (self.next() % 10_000) as f32 / 10_000.0 < p
    }
    pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len())]
    }
    pub fn digits_r(&mut self, min: usize, max: usize) -> String {
        let n = min + self.below(max - min + 1);
        self.digits(n)
    }
    pub fn digits(&mut self, n: usize) -> String {
        (0..n).map(|_| char::from(b'0' + self.below(10) as u8)).collect()
    }
}

const SEPS: &[&str] = &[
    "-", "-", "-", ".", " ", "_", "/", ":", ",", "|", "~", "*", "#", "+", "=", "\\", "--", " - ", ". ", "·", ";", "^", "'", "..",
    ")", "-_", " / ",
];

/// 묶음을 변형 구분자로 이어 붙인다. (일부 구분자 누락·혼합 포함)
fn obfuscate(r: &mut Rng, groups: &[String], allow_drop: bool) -> String {
    let uniform = r.chance(0.55);
    let base = r.pick(SEPS);
    let mut s = groups[0].clone();
    for g in &groups[1..] {
        let sep = if allow_drop && r.chance(0.18) {
            ""
        } else if uniform {
            base
        } else {
            r.pick(SEPS)
        };
        s.push_str(sep);
        s.push_str(g);
    }
    s
}

const FILLER: &[&str] = &[
    "", "", "", "data ", "값 ", "x=", "- ", "* ", "> ", "  ", "item: ", "note ", "메모 ", "비고 ", "\t", "// ", "# ", "<td>",
    "\"", "'", "[", "(", "value=\"", "기록 ", "row 3 ", "line 12: ",
];
const TAIL: &[&str] = &["", "", "\"", "'", ",", ";", " ", " 입니다", "</td>", ")", "]", " ok", "\r", " 끝", " end", "\",", " |"];

fn wrap(r: &mut Rng, ctx: &[&str], body: &str) -> (String, usize, usize) {
    let mut pre = String::new();
    if r.chance(0.3) {
        pre.push_str(r.pick(FILLER));
    }
    pre.push_str(r.pick(ctx));
    let post = r.pick(TAIL).to_string();
    let s = pre.len();
    let text = format!("{pre}{body}{post}");
    (text, s, s + body.len())
}

// ───────── 정답 생성 ─────────

fn mobile(r: &mut Rng) -> Vec<String> {
    if r.chance(0.85) {
        vec!["010".into(), format!("{}", r.range(2, 9)) + &r.digits(3), r.digits(4)]
    } else {
        let p = r.pick(&["011", "016", "017", "018", "019"]);
        let mid = if r.chance(0.5) { format!("{}", r.range(2, 9)) + &r.digits(2) } else { format!("{}", r.range(2, 9)) + &r.digits(3) };
        vec![p.into(), mid, r.digits(4)]
    }
}

fn luhn_complete(prefix: &str, len: usize, r: &mut Rng) -> String {
    let mut s = prefix.to_string() + &r.digits(len - prefix.len() - 1);
    for d in 0..10 {
        let cand = format!("{s}{d}");
        if crate::rules::dict::luhn(&cand) {
            s = cand;
            return s;
        }
    }
    unreachable!()
}

fn card(r: &mut Rng, valid: bool) -> String {
    let pre = r.pick(&["4", "51", "52", "53", "54", "55", "2221", "2720", "35"]);
    let s = luhn_complete(pre, 16, r);
    if valid {
        s
    } else {
        // 마지막 자리를 바꿔 Luhn 실패
        let last = (s.as_bytes()[15] - b'0' + 1 + r.below(8) as u8) % 10;
        format!("{}{}", &s[..15], last)
    }
}

fn rrn(r: &mut Rng, valid_date: bool) -> (String, String) {
    let y = r.range(0, 99);
    let (m, d) = if valid_date { (r.range(1, 12), r.range(1, 28)) } else { (r.range(13, 19), r.range(1, 28)) };
    let g = if y < 25 { r.pick(&[3, 4]) } else { r.pick(&[1, 2]) };
    let front = format!("{y:02}{m:02}{d:02}");
    let mut back = format!("{g}{}", r.digits(5));
    // 검증 자리
    let all = format!("{front}{back}");
    const W: [u32; 12] = [2, 3, 4, 5, 6, 7, 8, 9, 2, 3, 4, 5];
    let sum: u32 = all.bytes().zip(W).map(|(b, w)| (b - b'0') as u32 * w).sum();
    // 2020.10 이후 번호는 검증 자리 규칙이 없음 → 절반은 무작위
    let check = if r.chance(0.5) { ((11 - sum % 11) % 10) as u8 } else { r.below(10) as u8 };
    back.push(char::from(b'0' + check));
    (front, back)
}

const CTX_PHONE: &[&str] = &[
    "hp = \"", "연락처: ", "전화번호 ", "휴대폰 ", "phone: ", "tel=", "mobile ", "cell: ", "\"phone\": \"", "담당자 홍길동 ", "call ",
    "번호는 ", "HP)", "핸드폰번호:", "contact=", "문의 ", "H.P ", "연락 ", "", "", "", "",
];
const CTX_IP: &[&str] = &[
    "ip=", "IP: ", "host ", "server = \"", "addr ", "서버 ", "접속 ", "gateway ", "src=", "dst=", "from ", "remote_addr: ",
    "Allow from ", "ping ", "-> ", "inet ", "bind ", "서버 주소 ", "",
];
const CTX_CARD: &[&str] = &["카드번호 ", "card: ", "cc=", "결제 카드 ", "\"pan\": \"", "VISA ", "신용카드:", "", ""];
const CTX_RRN: &[&str] = &["주민번호 ", "주민등록번호: ", "rrn=", "ssn: ", "jumin=", "외국인등록번호 ", "생년월일 ", "", ""];
const CTX_NEUTRAL: &[&str] = &["", "", "no ", "번호 ", "id=", "값: ", "code ", "ref ", "#"];

/// 표본 1개 생성 → (텍스트, 정답 시작, 끝, 클래스)
fn positive(r: &mut Rng) -> (String, usize, usize, usize) {
    match r.below(10) {
        0..=3 => {
            let g = mobile(r);
            let body = if r.chance(0.12) {
                let sp = if r.chance(0.5) { " " } else { "" };
                format!("({}){sp}{}{}{}", g[0], g[1], r.pick(SEPS), g[2])
            } else {
                obfuscate(r, &g, true)
            };
            // 키워드가 바로 붙은 경우 (tel010-…)
            let ctx: &[&str] = if r.chance(0.1) { &["tel", "hp", "HP", "phone", "Tel."] } else { CTX_PHONE };
            let (t, s, e) = wrap(r, ctx, &body);
            (t, s, e, C_PHONE)
        }
        4..=6 => {
            let o = ipdict::sample_real(r);
            let g: Vec<String> = o.iter().map(|v| v.to_string()).collect();
            let body = obfuscate(r, &g, false);
            let with_ctx = r.chance(0.7);
            let ctx: &[&str] = if with_ctx { CTX_IP } else { CTX_NEUTRAL };
            let (t, s, e) = wrap(r, ctx, &body);
            let inf = ipdict::info(&o);
            // 사람의 판단: 맥락이 없어도 흔한 사설망·잘 알려진 서버는 IP 로 읽힌다.
            // 맥락 없는 일반 공인 대역은 다른 해석(코드·번호)의 여지가 있어 애매 (60%)
            let label = if with_ctx || inf.common_private || inf.well_known || r.chance(0.6) { C_IP } else { C_NONE };
            (t, s, e, label)
        }
        7..=8 => {
            let c = card(r, true);
            let groups: Vec<String> = c.as_bytes().chunks(4).map(|x| String::from_utf8_lossy(x).into()).collect();
            let body = obfuscate(r, &groups, true);
            let (t, s, e) = wrap(r, CTX_CARD, &body);
            (t, s, e, C_CARD)
        }
        _ => {
            let (f, b) = rrn(r, true);
            let body = format!("{f}{}{b}", r.pick(SEPS));
            let (t, s, e) = wrap(r, CTX_RRN, &body);
            (t, s, e, C_RRN)
        }
    }
}

const CTX_DATE: &[&str] = &["date: ", "작성일 ", "created_at=", "", "기간 ", "일시: ", "updated ", "timestamp=", "[", ""];
const CTX_VER: &[&str] = &["version ", "ver ", "build ", "v", "rev ", "버전 ", "release-"];
const CTX_MONEY: &[&str] = &["금액 ", "price: ", "$", "₩", "합계 ", "total=", ""];
const CTX_TABLE: &[&str] = &["", "rgb(", "color ", "mask ", "size ", "좌표 ", "lat/lng ", "<td>", "| "];
const CTX_ID: &[&str] = &["계좌 ", "account: ", "주문번호 ", "order=", "serial ", "사업자번호 ", "ISBN ", "송장 ", "", "모델 "];

fn negative(r: &mut Rng) -> (String, usize, usize) {
    let body = match r.below(16) {
        14 | 15 => {
            // 날짜 범위 (월.일 ~ 월.일) — 10월 날짜는 10.x 사설 IP 와 모양이 같다
            let m1 = if r.chance(0.4) { 10 } else { r.range(1, 12) };
            let (d1, m2, d2) = (r.range(1, 28), if r.chance(0.7) { m1 } else { r.range(1, 12) }, r.range(1, 31));
            let dot = r.pick(&[".", "/", "-"]);
            let tail = if dot == "." && r.chance(0.6) { "." } else { "" };
            let range = r.pick(&["~", ".~", " ~ ", "~ ", "-", " - ", "～"]);
            let range = if dot == "." && tail == "." && range == "~" { ".~" } else { range };
            let s = format!("{m1}{dot}{d1:02}{range}{m2}{dot}{d2:02}{tail}");
            let ctx: &[&str] = if r.chance(0.6) { &["(일정) ", "일정: ", "기간 ", "schedule ", "작업 ", "마감 ", "", "회의 "] } else { CTX_NEUTRAL };
            return wrap(r, ctx, &s);
        }
        0 | 1 => {
            // 날짜(+시각)
            let (y, m, d) = (r.range(1990, 2030), r.range(1, 12), r.range(1, 28));
            let sep = r.pick(&["-", ".", "/", " ", "_", ""]);
            let mut s = if r.chance(0.2) { format!("{d:02}{sep}{m:02}{sep}{y}") } else { format!("{y}{sep}{m:02}{sep}{d:02}") };
            if r.chance(0.5) {
                s += &format!("{}{:02}:{:02}:{:02}", r.pick(&[" ", "T", "-", "_"]), r.range(0, 23), r.range(0, 59), r.range(0, 59));
            }
            return wrap(r, CTX_DATE, &s);
        }
        2 => format!("{:02}:{:02}:{:02}.{}", r.range(0, 23), r.range(0, 59), r.range(0, 59), r.digits(3)),
        3 => {
            let s = format!("{}{}{}{}{}{}{}", r.range(0, 20), r.pick(&["-", "_", ".", ","]), r.range(0, 30), r.pick(&["-", "_"]), r.range(0, 30000), r.pick(&["-", "_"]), r.range(0, 9999));
            return wrap(r, CTX_VER, &s);
        }
        4 => {
            let n = r.range(1000, 999_999_999).to_string();
            let mut s = String::new();
            for (i, c) in n.chars().enumerate() {
                if i > 0 && (n.len() - i) % 3 == 0 {
                    s.push(',');
                }
                s.push(c);
            }
            let unit = r.pick(&["원", "", " KRW", ".00"]);
            return wrap(r, CTX_MONEY, &(s + unit));
        }
        5 => {
            // 표/색상/마스크 숫자 (공백·쉼표 구분)
            let n = r.range(4, 6) as usize;
            let sep = r.pick(&[" ", ", ", ",", "\t", " | "]);
            let s = (0..n).map(|_| r.range(0, 255).to_string()).collect::<Vec<_>>().join(sep);
            return wrap(r, CTX_TABLE, &s);
        }
        6 => {
            // 계좌·사업자·주문·ISBN 등 일반 번호
            match r.below(5) {
                0 => format!("{}-{}-{}", r.digits(3), r.digits_r(2, 4), r.digits(6)),
                1 => format!("{}-{}-{}", r.digits(3), r.digits(2), r.digits(5)),
                2 => format!("{}{:02}{:02}-{}", r.range(2015, 2030), r.range(1, 12), r.range(1, 28), r.digits(4)),
                3 => format!("978-89-{}-{}-{}", r.digits(4), r.digits(3), r.digits(1)),
                _ => format!("{}-{}-{}-{}", r.digits(4), r.digits(4), r.digits(4), r.digits(2)),
            }
        }
        7 => {
            // 일반전화 (휴대폰 아님)
            let a = r.pick(&["02", "031", "032", "051", "053", "062", "070", "080", "1588", "1544"]);
            format!("{a}{}{}{}{}", r.pick(SEPS), r.digits_r(3, 4), r.pick(SEPS), r.digits(4))
        }
        8 => {
            // 휴대폰 모양이지만 잘못된 번호
            let p = r.pick(&["012", "013", "014", "015", "019", "010"]);
            let mid = if p == "010" { format!("0{}", r.digits(3)) } else { r.digits(4) };
            format!("{p}{}{mid}{}{}", r.pick(SEPS), r.pick(SEPS), r.digits(4))
        }
        9 => {
            // Luhn 실패 카드 모양
            let c = card(r, false);
            let groups: Vec<String> = c.as_bytes().chunks(4).map(|x| String::from_utf8_lossy(x).into()).collect();
            obfuscate(r, &groups, true)
        }
        10 => {
            // 날짜가 틀린 주민번호 모양
            let (f, b) = rrn(r, false);
            format!("{f}{}{b}", r.pick(SEPS))
        }
        11 => {
            // 버전 맥락의 IP 모양
            let s = format!("{}.{}.{}.{}", r.range(0, 30), r.range(0, 30), r.range(0, 999), r.range(0, 99));
            let s = s.replace('.', r.pick(&["-", "_", ".", ","]));
            return wrap(r, CTX_VER, &s);
        }
        12 => {
            // 영문이 붙은 코드/해시 (구분자 모양은 맞음)
            let g = mobile(r);
            let pre = r.pick(&["a", "x", "f", "id", "ab", "0x"]);
            format!("{pre}{}{}{}", obfuscate(r, &g, true), r.pick(&["", "x", "f"]), "")
        }
        _ => {
            // 무작위 묶음 (IP 모양 4묶음≤255 는 제외)
            loop {
                let n = 2 + r.below(4);
                let gs: Vec<String> = (0..n).map(|_| r.digits_r(1, 5)).collect();
                let ip_like = n == 4 && gs.iter().all(|g| g.len() <= 3);
                if !ip_like {
                    break obfuscate(r, &gs, false);
                }
            }
        }
    };
    let ctx: &[&str] = if r.chance(0.4) { CTX_ID } else { CTX_NEUTRAL };
    wrap(r, ctx, &body)
}

/// 정답 위치가 표시된 합성 문서
#[derive(Default)]
struct Doc {
    text: String,
    /// (시작, 끝, 클래스)
    spans: Vec<(usize, usize, usize)>,
}

impl Doc {
    /// 한 줄 추가. (s, e) 는 줄 안에서의 정답 범위
    fn line(&mut self, text: &str, s: usize, e: usize, label: usize) {
        let base = self.begin_line();
        self.text.push_str(text);
        self.mark(base, text, s, e, label);
    }
    fn begin_line(&mut self) -> usize {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.len()
    }
    /// 정답 범위를 숫자 경계로 맞춰 기록 ("(010)…" 의 괄호, 붙은 영문 제외)
    fn mark(&mut self, base: usize, text: &str, s: usize, e: usize, label: usize) {
        let span = &text[s..e];
        if let (Some(a), Some(z)) = (span.find(|c: char| c.is_ascii_digit()), span.rfind(|c: char| c.is_ascii_digit())) {
            self.spans.push((base + s + a, base + s + z + 1, label));
        }
    }
    fn plain(&mut self, text: &str) {
        self.begin_line();
        self.text.push_str(text);
    }
}

fn fmt_ip(o: &[u32; 4], sep: &str) -> String {
    o.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(sep)
}

const NAMES: &[&str] = &["kim", "lee", "park", "admin", "root", "hong", "user01", "choi"];

/// 유출 로그: 한 가지 규칙(구분자)으로 문서 전체의 IP·전화번호를 일괄 변환한 경우
fn leak_log(r: &mut Rng, d: &mut Doc) {
    let sep = r.pick(&["-", "-", "_", " ", "/", ":", ",", "|", "~", "*", "#", "+", "--", "^"]);
    let n = 2 + r.below(5);
    for _ in 0..n {
        // 같은 규칙으로 바뀐 로그 안에 섞인 사람이 만든 배열 (12-34-56-78) — 주변이 IP 여도 IP 로 보지 않는다
        // (GUI 미리보기에서 발견: 문서 단위 흔적 특징이 부자연스러움 판단을 덮어씀)
        if r.chance(0.2) {
            let a = intuition::sample_artificial(r);
            let body = fmt_ip(&a, sep);
            let pre = r.pick(&["login from ", "server ", "host=", "code ", "src=", "node ", "", "ip "]);
            d.line(&format!("{pre}{body} ok"), pre.len(), pre.len() + body.len(), C_NONE);
        }
        let o = ipdict::sample_real(r);
        let ip = fmt_ip(&o, sep);
        let g = mobile(r);
        let ph = if r.chance(0.2) { format!("{}{}{sep}{}", g[0], g[1], g[2]) } else { g.join(sep) };
        let ts = format!(
            "2024-{:02}-{:02} {:02}:{:02}:{:02}",
            r.range(1, 12),
            r.range(1, 28),
            r.range(0, 23),
            r.range(0, 59),
            r.range(0, 59)
        );
        let name = r.pick(NAMES);
        match r.below(6) {
            0 => {
                let pre = format!("{ts} login from ");
                d.line(&format!("{pre}{ip} user {name}"), pre.len(), pre.len() + ip.len(), C_IP);
            }
            1 => {
                let o2 = ipdict::sample_real(r);
                let ip2 = fmt_ip(&o2, sep);
                let t = format!("src={ip} dst={ip2} port {}", r.range(1, 65535));
                let base = d.begin_line();
                d.text.push_str(&t);
                d.mark(base, &t, 4, 4 + ip.len(), C_IP);
                let s2 = 4 + ip.len() + 5;
                d.mark(base, &t, s2, s2 + ip2.len(), C_IP);
            }
            2 => {
                let pre = format!("{name}\t");
                d.line(&format!("{pre}{ph}\t개발팀"), pre.len(), pre.len() + ph.len(), C_PHONE);
            }
            3 => {
                d.line(&format!("{ip} - - [13/Nov/2024] \"GET / HTTP/1.1\" 200"), 0, ip.len(), C_IP);
            }
            4 => {
                let pre = r.pick(&["hp: ", "연락처 ", "tel ", ""]);
                d.line(&format!("{pre}{ph}"), pre.len(), pre.len() + ph.len(), C_PHONE);
            }
            _ => {
                let pre = r.pick(&["server ", "host=", "", "node "]);
                d.line(&format!("{pre}{ip} ok"), pre.len(), pre.len() + ip.len(), C_IP);
            }
        }
    }
}

/// 사람이 만든 배열 (12-34-56-78, 10-20-30-40 …) — IP 맥락이 있어도 대부분 예시·더미로 읽힌다
fn artificial(r: &mut Rng, d: &mut Doc) {
    let sep = r.pick(&["-", "-", "_", "/", " ", ":", "|", ","]);
    let n = 1 + r.below(3);
    let ip_ctx = r.chance(0.25);
    for _ in 0..n {
        let o = intuition::sample_artificial(r);
        let groups: Vec<String> = o.iter().map(|v| v.to_string()).collect();
        let body = if r.chance(0.8) { fmt_ip(&o, sep) } else { obfuscate(r, &groups, false) };
        let ctx: &[&str] = if ip_ctx {
            CTX_IP
        } else if r.chance(0.5) {
            CTX_ID
        } else {
            CTX_NEUTRAL
        };
        let (t, s, e) = wrap(r, ctx, &body);
        let label = if ip_ctx && r.chance(0.3) { C_IP } else { C_NONE };
        d.line(&t, s, e, label);
    }
}

/// 맥락 없는 무작위 4묶음 (57-203-14-98): 해석이 여럿 → 대부분 애매
fn random_quad(r: &mut Rng, d: &mut Doc) {
    let o = [r.range(1, 223), r.range(0, 255), r.range(0, 255), r.range(1, 254)];
    let sep = r.pick(SEPS);
    let body = fmt_ip(&o, sep);
    let k = r.below(3);
    let ctx: &[&str] = match k {
        0 => CTX_IP,
        1 => CTX_ID,
        _ => CTX_NEUTRAL,
    };
    let (t, s, e) = wrap(r, ctx, &body);
    let label = match k {
        0 if r.chance(0.85) => C_IP,
        2 if r.chance(0.4) => C_IP,
        _ => C_NONE,
    };
    d.line(&t, s, e, label);
}

fn document(r: &mut Rng) -> Doc {
    let mut d = Doc::default();
    match r.below(100) {
        0..=34 => {
            let (t, s, e, l) = positive(r);
            d.line(&t, s, e, l);
            if r.chance(0.3) {
                let (t, s, e) = negative(r);
                d.line(&t, s, e, C_NONE);
            }
        }
        35..=52 => leak_log(r, &mut d),
        53..=64 => artificial(r, &mut d),
        65..=73 => random_quad(r, &mut d),
        74..=89 => {
            for _ in 0..1 + r.below(3) {
                let (t, s, e) = negative(r);
                d.line(&t, s, e, C_NONE);
            }
        }
        _ => {
            // 혼합: 정상 표기(점) IP + 변형 로그 + 함정
            let o = ipdict::sample_real(r);
            let ip = fmt_ip(&o, ".");
            d.line(&format!("gateway {ip}"), 8, 8 + ip.len(), C_IP);
            leak_log(r, &mut d);
            let (t, s, e) = negative(r);
            d.line(&t, s, e, C_NONE);
        }
    }
    if r.chance(0.2) {
        let tail = r.pick(&["", "# 끝", "-----", "합계: 3건", "page 1/2"]);
        d.plain(tail);
    }
    d
}

/// 본 모델(2단) 표본 생성 — 문서의 모든 후보에 라벨 (정답 위치와 정확히 같으면 정답 클래스, 아니면 none)
pub fn dataset(seed: u64, n_docs: usize, nat: &gbdt::Model) -> (Vec<Vec<f32>>, Vec<usize>) {
    let mut r = Rng::new(seed);
    let mut x = Vec::new();
    let mut y = Vec::new();
    for _ in 0..n_docs {
        let d = document(&mut r);
        for (c, v) in features::doc_features(&d.text, nat) {
            let label = d.spans.iter().find(|&&(s, e, _)| s == c.start && e == c.end).map_or(C_NONE, |t| t.2);
            x.push(v);
            y.push(label);
        }
    }
    (x, y)
}

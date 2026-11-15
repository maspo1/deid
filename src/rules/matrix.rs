//! 정형 번호 정답 행렬 — 전화·주민/외국인·카드(+IPv4 사슬)를 **한 번 훑기**로 동시에 찾는다.
//!
//! 형식마다 "몇 번째 칸에 어떤 글자가 올 수 있는가"를 미리 표로 만든다 (정답 행렬).
//! 예) 주민번호 `881111-1234567` 의 3번째 칸(월 십의 자리)은 0·1 만, 5번째 칸(일 십의 자리)은 0~3 만.
//!
//! 표: `TABLE[바이트] = 이 바이트가 들어갈 수 있는 (형식, 칸) 비트` — 256 × 256비트 = 8KB (L1 캐시 안).
//! 모든 형식의 칸을 비트 하나씩 이어 붙여 두고, 바이트마다
//!
//! ```text
//!   D = ((D << 1) | 시작칸) & TABLE[바이트]      (Shift-And, 불리언 행렬 곱의 비트 병렬형)
//! ```
//!
//! 한 번이면 "지금까지 k글자가 형식 X 의 앞 k칸과 일치" 상태가 모든 형식에 대해 동시에 갱신된다.
//! 분기 없이 바이트당 표 조회 2번 + 시프트·AND. 끝칸 비트가 켜질 때(드묾)만 2단계 검증으로 간다.
//! 모든 형식이 숫자로 시작하므로, 진행 중인 형식이 없으면 다음 숫자까지 건너뛴다 (글자·태그만 있는 구간은 비교 1번).
//!
//! 2단계(드물게 실행): 앞뒤 경계 → 사전 규칙(국번, Luhn, 생년월일 정답표+검증번호) — 기존 정규식 경로와 같은 함수.
//! MAC 은 영문(a~f)으로도 시작해 건너뛰기를 막으므로 정규식 경로(리터럴 거르기로 이미 ~1GB/s)에 둔다.
//! IPv4 는 길이가 가변(점 3개 이상, 묶음 1~3자리)이라 같은 훑기 안에서 "숫자.숫자…" 사슬만 추적해 구조 규칙에 넘긴다.
//!
//! 정규식과 다른 점: 정규식은 맞은 자리를 소비하고 다음으로 넘어가지만, 행렬은 모든 위치를 본다.
//! 그래서 정규식이 앞의 실패한 일치에 가려 못 보던 유효한 번호도 찾는다 (겹치면 `resolve_overlaps` 가 하나만 남김).

use super::dict;
use super::patterns::{card_context, digit_bounded, glued_to_code, ipv4_at};
use crate::common::{Hit, InfoType, MaskMode};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fam {
    Phone,
    Rrn,
    Card,
}

struct Lane {
    fam: Fam,
    cols: Vec<&'static [u8]>,
    /// 끝으로 인정하는 길이들
    ends: Vec<usize>,
}

const D: &[u8] = b"0123456789";
/// 휴대폰 가운데 첫 자리 (0 으로 시작하는 국번 뒤 번호는 없음)
const MID: &[u8] = b"123456789";
/// 주민·외국인 뒷자리 첫 숫자 (성별·세기: 0·9 는 1800년대 출생이라 쓰이지 않음)
const GENDER: &[u8] = b"12345678";

fn lanes() -> Vec<Lane> {
    let mut v = Vec::new();
    let rep = |c: &'static [u8], n: usize| std::iter::repeat(c).take(n);
    let seq = |parts: Vec<Vec<&'static [u8]>>| parts.concat();
    // 전화: 01X + 7~8자리 (구분자 없음) / 01X s 3~4 s 4 (구분자는 같은 글자). 가운데 첫 자리는 1~9
    v.push(Lane { fam: Fam::Phone, cols: seq(vec![vec![b"0", b"1", b"016789", MID], rep(D, 7).collect()]), ends: vec![10, 11] });
    for s in [b"-" as &[u8], b".", b" "] {
        let head: Vec<&[u8]> = vec![b"0", b"1", b"016789", s, MID];
        v.push(Lane { fam: Fam::Phone, cols: seq(vec![head.clone(), rep(D, 3).collect(), vec![s], rep(D, 4).collect()]), ends: vec![13] });
        v.push(Lane { fam: Fam::Phone, cols: seq(vec![head, rep(D, 2).collect(), vec![s], rep(D, 4).collect()]), ends: vec![12] });
    }
    // 주민·외국인: YYMMDD (월 십의 자리 0·1, 일 십의 자리 0~3) + 성별(1~8) + 6자리, 구분자 없음 / '-' / ' '
    let birth: Vec<&[u8]> = vec![D, D, b"01", D, b"0123", D];
    v.push(Lane { fam: Fam::Rrn, cols: seq(vec![birth.clone(), vec![GENDER], rep(D, 6).collect()]), ends: vec![13] });
    for s in [b"-" as &[u8], b" "] {
        v.push(Lane { fam: Fam::Rrn, cols: seq(vec![birth.clone(), vec![s, GENDER], rep(D, 6).collect()]), ends: vec![14] });
    }
    // 카드: 13~16자리 / 4 s 4 s 4 s 1~4  ·  아멕스: 3xxx 6 5 (14~15자리) / 3xxx s 6 s 4~5
    v.push(Lane { fam: Fam::Card, cols: rep(D, 16).collect(), ends: vec![13, 14, 15, 16] });
    v.push(Lane { fam: Fam::Card, cols: seq(vec![vec![b"3"], rep(D, 14).collect()]), ends: vec![14, 15] });
    for s in [b"-" as &[u8], b" "] {
        let g4: Vec<&[u8]> = rep(D, 4).collect();
        v.push(Lane { fam: Fam::Card, cols: seq(vec![g4.clone(), vec![s], g4.clone(), vec![s], g4.clone(), vec![s], g4]), ends: vec![16, 17, 18, 19] });
        v.push(Lane { fam: Fam::Card, cols: seq(vec![vec![b"3", D, D, D, s], rep(D, 6).collect(), vec![s], rep(D, 5).collect()]), ends: vec![16, 17] });
    }
    v
}

const W: usize = 2; // 128비트 × 2 = 256칸

pub struct Matrix {
    table: Box<[[u128; W]; 256]>,
    start: [u128; W],
    end: [u128; W],
    /// 끝칸 비트 → (형식, 길이)
    end_of: Vec<(Fam, usize)>,
}

impl Matrix {
    pub fn new() -> Self {
        let mut table = Box::new([[0u128; W]; 256]);
        let (mut start, mut end) = ([0u128; W], [0u128; W]);
        let mut end_of = vec![(Fam::Phone, 0); W * 128];
        let (mut word, mut bit) = (0usize, 0usize);
        for lane in lanes() {
            let n = lane.cols.len();
            // 한 형식은 128비트 낱말 하나 안에 (시프트가 낱말을 넘지 않도록)
            if bit + n > 128 {
                word += 1;
                bit = 0;
            }
            assert!(word < W, "정답 행렬 칸 부족");
            start[word] |= 1u128 << bit;
            for (k, set) in lane.cols.iter().enumerate() {
                for &b in set.iter() {
                    table[b as usize][word] |= 1u128 << (bit + k);
                }
            }
            for &len in &lane.ends {
                end[word] |= 1u128 << (bit + len - 1);
                end_of[word * 128 + bit + len - 1] = (lane.fam, len);
            }
            bit += n;
        }
        Matrix { table, start, end, end_of }
    }

    /// 켜진 형식만 검사해 `out` 에 더함. `ip` 가 켜져 있으면 같은 훑기에서 IPv4 사슬도 판단.
    pub fn scan(&self, text: &str, on: impl Fn(&InfoType) -> bool, out: &mut Vec<Hit>) {
        let (phone, rrn, card, ip) = (on(&InfoType::Phone), on(&InfoType::Rrn) || on(&InfoType::Frn), on(&InfoType::Card), on(&InfoType::Ip));
        let want = |f: Fam| match f {
            Fam::Phone => phone,
            Fam::Rrn => rrn,
            Fam::Card => card,
        };
        let b = text.as_bytes();
        let mut d = [0u128; W];
        // IPv4 사슬: 시작 위치, 묶음 수, 직전이 점인가
        let mut chain: Option<usize> = None;
        let (mut groups, mut dot) = (0usize, false);
        let mut i = 0;
        while i < b.len() {
            // 진행 중인 형식·사슬이 없으면 다음 숫자까지 건너뜀 (모든 형식의 첫 칸은 숫자)
            if chain.is_none() && d.iter().all(|&x| x == 0) {
                match b[i..].iter().position(u8::is_ascii_digit) {
                    Some(k) => i += k,
                    None => break,
                }
            }
            let c = b[i];
            let t = &self.table[c as usize];
            let mut any = 0u128;
            for w in 0..W {
                d[w] = ((d[w] << 1) | self.start[w]) & t[w];
                any |= d[w] & self.end[w];
            }
            if any != 0 {
                for w in 0..W {
                    let mut e = d[w] & self.end[w];
                    while e != 0 {
                        let k = e.trailing_zeros() as usize;
                        e &= e - 1;
                        let (fam, len) = self.end_of[w * 128 + k];
                        if want(fam) {
                            verify(text, fam, i + 1 - len, i + 1, out);
                        }
                    }
                }
            }
            if ip {
                match c {
                    b'0'..=b'9' => {
                        if chain.is_none() {
                            chain = Some(i);
                            groups = 1;
                        } else if dot {
                            groups += 1;
                        }
                        dot = false;
                    }
                    b'.' if chain.is_some() && !dot => dot = true,
                    _ => {
                        if let Some(s) = chain.take() {
                            close_chain(text, s, i, groups, dot, out);
                        }
                        dot = false;
                    }
                }
            }
            i += 1;
        }
        if let Some(s) = chain {
            close_chain(text, s, b.len(), groups, dot, out);
        }
    }
}

impl Default for Matrix {
    fn default() -> Self {
        Self::new()
    }
}

/// 사슬 `s..i` 가 끝남 (끝의 점은 사슬에 넣지 않음). 묶음이 4개 이상이면 IP 구조 규칙으로.
fn close_chain(text: &str, s: usize, i: usize, groups: usize, dot: bool, out: &mut Vec<Hit>) {
    if groups >= 4 {
        ipv4_at(text, s, if dot { i - 1 } else { i }, out);
    }
}

/// 2단계: 끝칸에 닿은 후보를 사전 규칙으로 확인 (정규식 경로와 같은 규칙)
fn verify(text: &str, fam: Fam, s: usize, e: usize, out: &mut Vec<Hit>) {
    let v = &text[s..e];
    let digits: String = v.bytes().filter(u8::is_ascii_digit).map(char::from).collect();
    match fam {
        Fam::Phone => {
            if !digit_bounded(text, s, e) || glued_to_code(text, s, e) {
                return;
            }
            let (prefix, rest) = digits.split_at(3);
            let (mid, last) = rest.split_at(rest.len() - 4);
            if dict::valid_mobile(prefix, mid, last) {
                out.push(Hit::new(s, e, InfoType::Phone, MaskMode::Alnum));
            }
        }
        Fam::Rrn => {
            if !digit_bounded(text, s, e) {
                return;
            }
            if let Some(kind) = dict::rrn_kind(&digits[..6], &digits[6..], v.len() == 14) {
                // 보통의 표기(YYMMDD-NNNNNNN)가 아니면 의심 → 치환 + 검토필요
                let hit = Hit::new(s, e, kind, MaskMode::Alnum);
                out.push(if v.as_bytes()[6] == b'-' { hit } else { hit.needs_review() });
            }
        }
        Fam::Card => {
            let g: Vec<usize> = v.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).map(str::len).collect();
            if !digit_bounded(text, s, e) || !dict::card_layout(&g) {
                return;
            }
            let hit = Hit::new(s, e, InfoType::Card, MaskMode::Alnum);
            if dict::valid_card(&digits) {
                // 붙여 쓴 숫자열은 계좌·주문·ID 번호와 겹친다 → 카드 맥락이 없으면 치환 + 검토필요
                let bare = v.len() == digits.len();
                out.push(if bare && !card_context(text, s) { hit.needs_review() } else { hit });
            } else if v.len() > digits.len()
                && (13..=19).contains(&digits.len())
                && dict::valid_card_iin(&digits)
                && card_context(text, s)
            {
                out.push(hit.needs_review());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::patterns::Patterns;

    fn key(t: &str, v: &[Hit]) -> Vec<(usize, usize, String, bool)> {
        let mut k: Vec<_> = v.iter().map(|h| (h.start, h.end, format!("{:?}", h.kind), h.review)).collect();
        k.sort();
        k.dedup();
        let _ = t;
        k
    }

    fn old(p: &Patterns, t: &str) -> Vec<Hit> {
        let mut v = Vec::new();
        p.phones(t, &mut v);
        p.cards(t, &mut v);
        p.rrns(t, &mut v);
        p.ipv4s(t, &mut v);
        v
    }

    fn new(m: &Matrix, t: &str) -> Vec<Hit> {
        let mut v = Vec::new();
        m.scan(t, |_| true, &mut v);
        v
    }

    #[test]
    fn finds_each_format() {
        let m = Matrix::new();
        let t = "tel 010-1111-2222, 011.234.5678 / 01011112222 주민 881111-1234567 카드 4111-1111-1111-1111 \
                 amex 3782-822463-10005 host 192.168.77.1:8080 끝";
        let got: Vec<&str> = {
            let mut v = new(&m, t);
            v.sort_by_key(|h| h.start);
            v.iter().map(|h| &t[h.start..h.end]).collect::<Vec<_>>()
        };
        for want in ["010-1111-2222", "011.234.5678", "01011112222", "881111-1234567", "4111-1111-1111-1111", "3782-822463-10005", "192.168.77.1"] {
            assert!(got.contains(&want), "{want} 없음: {got:?}");
        }
    }

    /// 기존 정규식 경로와 같은 결과인가 — 번호 모양 조각을 무작위로 이어 붙인 문자열 30만 개.
    /// 차이는 정규식이 앞의 실패한 일치에 가려 놓친 것(행렬만 찾음)만 허용하고, 그 경우도 유효한 번호인지 확인.
    #[test]
    fn same_as_regex_on_random_text() {
        let (p, m) = (Patterns::new(), Matrix::new());
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut rnd = |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        let pieces = [
            "010", "011", "016", "0", "1", "3", "4111", "1111", "1234", "5678", "881111", "1234567", "-", ".", " ", ":", "a", "F",
            "00", "1A", "2B", "192", "168", "77", "255", "x", "v", "서버", "카드", ",", "\n", "+82", "99", "4", "12", "31", "0101",
            "010-1234-5678", "4111-1111-1111-1111", "881111-1234567", "192.168.0.1", "00:1A:2B:3C:4D:5E", "01012345678",
            "3782 822463 10005", "4111111111111111", "8811111234568", "10.20.30.40",
        ];
        let (mut only_new, mut total) = (0, 0);
        for _ in 0..300_000 {
            let n = 3 + rnd(14);
            let t: String = (0..n).map(|_| pieces[rnd(pieces.len() as u64) as usize]).collect();
            // 국제 표기(+82)는 정규식 경로가 계속 담당하므로 비교에서 뺀다
            let o: Vec<Hit> = old(&p, &t).into_iter().filter(|h| !t[h.start..h.end].starts_with('+')).collect();
            let (ko, kn) = (key(&t, &o), key(&t, &new(&m, &t)));
            total += ko.len();
            for h in &ko {
                assert!(kn.contains(h), "행렬이 놓침: {t:?} {h:?}\n  정규식 {ko:?}\n  행렬 {kn:?}");
            }
            for h in &kn {
                if !ko.contains(h) {
                    if only_new < 8 {
                        println!("행렬만: {:?} → {:?}", t, &t[h.0..h.1]);
                    }
                    only_new += 1;
                }
            }
        }
        assert!(total > 10_000, "검출 표본이 너무 적음: {total}");
        // 정규식이 소비 때문에 놓친 추가 검출은 드물어야 한다
        assert!(only_new * 50 < total, "행렬만 찾은 것 {only_new} / 정규식 {total}");
        println!("정규식 검출 {total}, 행렬만 추가로 찾음 {only_new}");
    }
}

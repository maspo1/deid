//! 탐지 성능 평가 (학습 후 재빌드해서 실행): `cargo run --release --example eval_ml`
//!
//! 실제 탐지기(`Detector`, 정답 행렬 + 사전 규칙 + ML)로 잰다.
//! 1. 정형 표기 (010-1234-5678, 881111-1234567, 4111-1111-1111-1111, 192.168.0.1) → 100% 확정 검출이어야 한다
//! 2. 기호가 바뀌거나 빠진 표기 (학습과 다른 seed) → 재현율 (치환 + 검토필요 포함)
//! 3. 함정 (날짜·버전·좌표·쪽 범위·파일명 …) → 오탐률
//! 4. 실제 문서에서 보고된 오답 → 0건이어야 한다
//! 5. 프로그램으로 변형한 레코드 묶음 (88^11^25*1,88&11$25*1) → 재현율

use deid::common::InfoType;
use deid::config::Options;
use deid::rules::dict;
use deid::rules::ml::features::{C_CARD, C_IP, C_PHONE, C_RRN};
use deid::rules::ml::synth::{eval_negative, eval_positive, eval_records, Rng};
use deid::rules::{Detector, Lang};

fn groups(span: &str) -> Vec<usize> {
    span.split(|c: char| !c.is_ascii_digit()).filter(|g| !g.is_empty()).map(str::len).collect()
}

/// 사전 규칙상 가능한 정보인가 (규칙으로 불가능한 표본은 재현율에서 뺀다)
fn possible(class: usize, span: &str) -> bool {
    let d: String = span.chars().filter(char::is_ascii_digit).collect();
    let g = groups(span);
    match class {
        C_PHONE => {
            let parts = match d.len() {
                11 => Some((&d[..3], &d[3..7], &d[7..])),
                10 => Some((&d[..3], &d[3..6], &d[6..])),
                _ => None,
            };
            parts.is_some_and(|(a, m, l)| dict::phone_layout(&g) && dict::valid_mobile(a, m, l))
        }
        C_RRN => d.len() == 13 && dict::rrn_layout(&g) && dict::rrn_kind(&d[..6], &d[6..], g.len() > 1).is_some(),
        C_CARD => dict::card_layout(&g) && dict::valid_card(&d),
        C_IP => {
            let ip = span.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(".");
            g.len() == 4 && g.iter().all(|&n| n <= 3) && dict::valid_ipv4(&ip) && !dict::ipv4_is_trivial(&ip)
        }
        _ => false,
    }
}

fn kind_matches(class: usize, k: &InfoType) -> bool {
    match class {
        C_PHONE => *k == InfoType::Phone,
        C_RRN => matches!(k, InfoType::Rrn | InfoType::Frn),
        C_CARD => *k == InfoType::Card,
        C_IP => *k == InfoType::Ip,
        _ => false,
    }
}

fn luhn_complete(prefix: &str, n: usize, r: &mut Rng) -> String {
    let mut s = prefix.to_string();
    while s.len() < n - 1 {
        s.push(char::from(b'0' + r.below(10) as u8));
    }
    (0..10).map(|c| format!("{s}{c}")).find(|x| dict::luhn(x)).unwrap()
}

/// 정형 표기 1개 → (텍스트, 시작, 끝, 클래스)
fn formal(r: &mut Rng) -> (String, usize, usize, usize) {
    let ctx = r.pick(&["", "", "연락처 ", "값: ", "no ", "담당 ", "<td>", "\"", "정보 "]);
    let (body, class) = match r.below(4) {
        0 => {
            let mid = format!("{}{}", 1 + r.below(9), r.digits(3));
            let last = loop {
                let l = r.digits(4);
                if l != "0000" {
                    break l;
                }
            };
            (format!("010-{mid}-{last}"), C_PHONE)
        }
        1 => loop {
            let (yy, mm, dd) = (r.range(30, 99), r.range(1, 12), r.range(1, 28));
            let g = 1 + r.below(2);
            let back = format!("{g}{}", r.digits(6));
            if dict::rrn_kind(&format!("{yy:02}{mm:02}{dd:02}"), &back, true).is_some() {
                break (format!("{yy:02}{mm:02}{dd:02}-{back}"), C_RRN);
            }
        },
        2 => {
            let c = luhn_complete(r.pick(&["4", "51", "55", "2221", "35"]), 16, r);
            let sep = r.pick(&["-", " "]);
            (format!("{}{sep}{}{sep}{}{sep}{}", &c[..4], &c[4..8], &c[8..12], &c[12..]), C_CARD)
        }
        _ => loop {
            let o = [r.range(1, 223), r.range(0, 255), r.range(0, 255), r.range(1, 254)];
            let ip = format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3]);
            if dict::valid_ipv4(&ip) && !dict::ipv4_is_trivial(&ip) {
                break (ip, C_IP);
            }
        },
    };
    let tail = r.pick(&["", " 입니다", "</td>", "\"", ", 끝"]);
    let t = format!("{ctx}{body}{tail}");
    (t, ctx.len(), ctx.len() + body.len(), class)
}

const NAMES: [&str; 5] = ["none", "phone", "ip", "card", "rrn"];

fn main() {
    // IP 변형은 IP주소(심층)에서만 찾으므로 심층을 켜고 잰다 (함정 오탐은 기본 설정도 함께)
    let mut deep = Options::default();
    deep.enabled.insert(InfoType::IpDeep);
    let det = Detector::new(deep);
    let plain = Detector::new(Options::default());
    let mut r = Rng::new(9_001);

    // 1. 정형 표기
    let (mut ok, mut total) = (0, 0);
    let mut miss = Vec::new();
    for _ in 0..20_000 {
        let (t, s, e, c) = formal(&mut r);
        total += 1;
        let hits = det.scan(&t, Lang::None);
        if hits.iter().any(|h| h.start == s && h.end == e && kind_matches(c, &h.kind) && !h.review) {
            ok += 1;
        } else if miss.len() < 5 {
            miss.push(t);
        }
    }
    println!("[1] 정형 표기 확정 검출: {ok}/{total} ({:.2}%)", 100.0 * ok as f64 / total as f64);
    for m in &miss {
        println!("    놓침: {m}");
    }

    // 2. 기호가 바뀌거나 빠진 표기
    let mut per = [[0usize; 3]; 5]; // [검출, 그중 확정, 전체]
    let mut ip_uniform = [0usize; 2]; // 구분자가 한 규칙인 IP: [검출, 전체]
    let mut miss = Vec::new();
    let mut r = Rng::new(9_002);
    for _ in 0..30_000 {
        let (t, s, e, c) = eval_positive(&mut r);
        if c == 0 || !possible(c, &t[s..e]) {
            continue;
        }
        per[c][2] += 1;
        let seps: Vec<&str> = t[s..e].split(|ch: char| ch.is_ascii_digit()).filter(|x| !x.is_empty()).collect();
        let uniform = c == C_IP && dict::uniform_sep(&seps).is_some();
        ip_uniform[1] += uniform as usize;
        let hits = det.scan(&t, Lang::None);
        match hits.iter().find(|h| h.start == s && h.end == e && kind_matches(c, &h.kind)) {
            Some(h) => {
                per[c][0] += 1;
                per[c][1] += !h.review as usize;
                ip_uniform[0] += uniform as usize;
            }
            None if miss.len() < 12 => miss.push(format!("{} : {}", NAMES[c], t)),
            None => {}
        }
    }
    println!("[2] 기호가 바뀌거나 빠진 표기 (규칙상 가능한 것만):");
    for c in 1..5 {
        let [found, sure, n] = per[c];
        println!("    {:<6} 검출 {found}/{n} ({:.1}%), 그중 확정 {sure}", NAMES[c], 100.0 * found as f64 / n.max(1) as f64);
    }
    let [found, n] = ip_uniform;
    println!("    └ ip 구분자가 한 규칙(아스키 코드 동일)인 것: 검출 {found}/{n} ({:.2}%), 놓침 {}", 100.0 * found as f64 / n.max(1) as f64, n - found);
    for m in &miss {
        println!("    놓침 {m}");
    }

    // 3. 함정
    let (mut fp, mut n, mut fp_plain) = (0, 0, 0);
    let mut shown = Vec::new();
    let mut r = Rng::new(9_003);
    for _ in 0..30_000 {
        let t = eval_negative(&mut r);
        n += 1;
        fp_plain += !plain.scan(&t, Lang::None).is_empty() as usize;
        let hits = det.scan(&t, Lang::None);
        if !hits.is_empty() {
            fp += 1;
            if shown.len() < 15 {
                shown.push(format!("{t}  →  {}", hits.iter().map(|h| format!("{}{}", &t[h.start..h.end], if h.review { "(검토)" } else { "" })).collect::<Vec<_>>().join(", ")));
            }
        }
    }
    println!("[3] 함정 오탐 (심층 켬): {fp}/{n} ({:.2}%)   기본 설정(IP 점 표기만): {fp_plain}/{n} ({:.2}%)", 100.0 * fp as f64 / n as f64, 100.0 * fp_plain as f64 / n as f64);
    for s in &shown {
        println!("    {s}");
    }

    // 4. 실제 문서에서 보고된 오답
    let reported = [
        "00013027665 14", "00013027665 7 8", "00013027665 3 4", "120712, 0.988423", "511.05667 -625.17676",
        "66669 -565.33331 608 -584", "44610,1903,2491,855278", "55117,932,81316,308", "6973025 322.1098442", "28-35, 97-98",
        "38-39, 55, 72", "37-44, 54-60", "68, 86, 106-109", "57, 88-89, 103", "104-105, 106-109", "205-1~205-3", "2.3.0.0",
        "129  4  22(21", "31.6→35.8", "152.1 145 150", "0.240.170.17", "880922000000", "8809220000000", "880922-0000000",
        "250117,250118,2502", "260116,260117,2601", "11.7.8.5.122.90.3.1.7.x", "122.90 -> 3.1", "image.png",
        "IMG_20240117_120712.png", "21.0.ㅇ.88.11.25.1", "21.0.88.11.25.1.7", "3^4*55-12^9*200-1", "103^234^126^56.7^8",
        "1 0 0 : 1", "011-234-5678", "019 1234 5678",
    ];
    let bad: Vec<String> = reported
        .iter()
        .filter_map(|t| {
            let hits = det.scan(t, Lang::None);
            (!hits.is_empty()).then(|| format!("{t} → {}", hits.iter().map(|h| &t[h.start..h.end]).collect::<Vec<_>>().join(", ")))
        })
        .collect();
    println!("[4] 보고된 오답 {}개 중 검출된 것: {}", reported.len(), bad.len());
    for b in &bad {
        println!("    {b}");
    }
    // 규칙상 형태는 맞아 검토필요로만 남겨야 하는 것
    for t in [
        "1110221213507",
        "39393717444875",
        "122.90.3.1",
        "서버 IP 122->90->3->1",
        "연락처 010→1234→5678",
        "88.11.25.1,14.20.1.3",
        "88^11^25*1,88&11$25*1",
        "[88^11^25*1][88&11$25*1]",
        "88-11-25-1;14-20-1-3;201-33-7-90",
        "192_168_0_1:8080",
    ] {
        let hits = det.scan(t, Lang::None);
        println!(
            "    참고 {t:<24} → {}",
            hits.iter().map(|h| format!("{:?}{} {}", h.kind, if h.review { "(검토)" } else { "" }, &t[h.start..h.end])).collect::<Vec<_>>().join(", ")
        );
    }

    // 5. 프로그램으로 변형한 레코드 묶음
    let (mut found, mut total, mut sure) = (0, 0, 0);
    let mut miss = Vec::new();
    let mut r = Rng::new(9_004);
    for _ in 0..10_000 {
        let (t, spans) = eval_records(&mut r);
        let hits = det.scan(&t, Lang::None);
        for (s, e, c) in spans {
            if !possible(c, &t[s..e]) {
                continue;
            }
            total += 1;
            match hits.iter().find(|h| h.start == s && h.end == e && kind_matches(c, &h.kind)) {
                Some(h) => {
                    found += 1;
                    sure += !h.review as usize;
                }
                None if miss.len() < 8 => miss.push(format!("{} : {t}", &t[s..e])),
                None => {}
            }
        }
    }
    println!("[5] 프로그램 변형 레코드 묶음: 검출 {found}/{total} ({:.1}%), 그중 확정 {sure}", 100.0 * found as f64 / total.max(1) as f64);
    for m in &miss {
        println!("    놓침 {m}");
    }
}

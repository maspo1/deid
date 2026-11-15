//! IP 사전 — 사설/특수 대역 사전과 공인 대역 사전.
//!
//! 학습(실제 IP 분포 생성)과 추론(특징)에 함께 쓴다.
//! * 사설·특수: RFC 1918 사설, CGNAT, 루프백, 링크로컬, **문서용 예시 대역**, 벤치마크, 멀티캐스트, 예약
//! * 실무에서 흔한 사설 서브넷: 192.168.0/1/10/100.x, 10.0.x, 172.16~31 (Docker 172.17, AWS VPC 172.31 …)
//! * 공인: IANA /8 할당표(지역 인터넷 레지스트리), 국내(KRNIC) 할당이 많은 /8, 주요 클라우드 /8, 잘 알려진 공용 서버(DNS)
//!
//! 공인 /8 표는 IANA IPv4 Address Space Registry 를 /8 단위로 요약한 것이다(세부 할당은 더 잘게 나뉨).
//! 국내·클라우드 목록은 대략적인 경향(사전 확률)용이며, 판정 규칙이 아니라 학습 특징으로만 쓴다.

/// /8 단위 레지스트리 분류
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registry {
    Reserved = 0,
    Private = 1,
    Loopback = 2,
    Multicast = 3,
    Legacy = 4,
    Arin = 5,
    Ripe = 6,
    Apnic = 7,
    Lacnic = 8,
    Afrinic = 9,
    Various = 10,
}

pub fn registry(o1: u32) -> Registry {
    use Registry::*;
    match o1 {
        0 => Reserved,
        10 => Private,
        127 => Loopback,
        224..=239 => Multicast,
        240..=255 => Reserved,
        1 | 14 | 27 | 36 | 39 | 42 | 43 | 49 | 58..=61 | 101 | 103 | 106 | 110..=126 | 175 | 180 | 182 | 183 | 202 | 203 | 210
        | 211 | 218..=223 => Apnic,
        2 | 5 | 31 | 37 | 46 | 62 | 77..=95 | 109 | 176 | 178 | 185 | 188 | 193..=195 | 212 | 213 | 217 => Ripe,
        41 | 102 | 105 | 154 | 196 | 197 => Afrinic,
        177 | 179 | 181 | 186 | 187 | 189..=191 | 200 | 201 => Lacnic,
        23 | 24 | 45 | 47 | 50 | 63..=76 | 96..=100 | 104 | 107 | 108 | 173 | 174 | 184 | 198 | 199 | 204..=209 | 216 => Arin,
        3 | 4 | 6..=9 | 11..=13 | 15..=22 | 25 | 26 | 28..=30 | 32..=35 | 38 | 40 | 44 | 48 | 51..=57 | 214 | 215 => Legacy,
        _ => Various,
    }
}

/// 국내(KRNIC) 할당 비중이 큰 /8 (대략)
pub const KR_HEAVY: &[u32] = &[
    1, 14, 27, 39, 42, 49, 58, 59, 61, 101, 106, 110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 121, 122, 123, 124, 125,
    175, 180, 182, 183, 203, 210, 211, 218, 219, 220, 221, 222,
];

/// 주요 클라우드(AWS/Azure/GCP/Cloudflare) 비중이 큰 /8 (대략)
pub const CLOUD: &[u32] = &[3, 13, 15, 18, 20, 34, 35, 40, 44, 52, 54, 104, 108, 130, 142, 172];

/// 잘 알려진 공용 서버 (DNS 등)
pub const WELL_KNOWN: &[[u32; 4]] = &[
    [8, 8, 8, 8],
    [8, 8, 4, 4],
    [1, 1, 1, 1],
    [1, 0, 0, 1],
    [9, 9, 9, 9],
    [149, 112, 112, 112],
    [208, 67, 222, 222],
    [208, 67, 220, 220],
    [4, 2, 2, 2],
    [168, 126, 63, 1], // KT
    [168, 126, 63, 2],
    [164, 124, 101, 2], // LG U+
    [164, 124, 107, 9],
    [210, 220, 163, 82], // SK브로드밴드
    [219, 250, 36, 130],
    [203, 248, 252, 2],
    [180, 182, 54, 1],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    /// RFC 1918 사설
    Private,
    /// 100.64/10 통신사 공유(CGNAT)
    Shared,
    Loopback,
    LinkLocal,
    /// 문서·예시용 (192.0.2/24, 198.51.100/24, 203.0.113/24) — 실제 장비가 아님
    Documentation,
    Benchmark,
    Multicast,
    Reserved,
}

pub struct Cidr {
    pub net: [u32; 4],
    pub len: u32,
    pub name: &'static str,
    pub kind: Special,
}

const fn c(a: u32, b: u32, cc: u32, d: u32, len: u32, name: &'static str, kind: Special) -> Cidr {
    Cidr { net: [a, b, cc, d], len, name, kind }
}

/// 사설·특수 대역 사전
pub const SPECIAL: &[Cidr] = &[
    c(0, 0, 0, 0, 8, "현재 네트워크", Special::Reserved),
    c(10, 0, 0, 0, 8, "사설 A", Special::Private),
    c(100, 64, 0, 0, 10, "CGNAT 공유", Special::Shared),
    c(127, 0, 0, 0, 8, "루프백", Special::Loopback),
    c(169, 254, 0, 0, 16, "링크 로컬", Special::LinkLocal),
    c(172, 16, 0, 0, 12, "사설 B", Special::Private),
    c(192, 0, 0, 0, 24, "IETF 프로토콜", Special::Reserved),
    c(192, 0, 2, 0, 24, "문서용 TEST-NET-1", Special::Documentation),
    c(192, 88, 99, 0, 24, "6to4 릴레이", Special::Reserved),
    c(192, 168, 0, 0, 16, "사설 C", Special::Private),
    c(198, 18, 0, 0, 15, "벤치마크", Special::Benchmark),
    c(198, 51, 100, 0, 24, "문서용 TEST-NET-2", Special::Documentation),
    c(203, 0, 113, 0, 24, "문서용 TEST-NET-3", Special::Documentation),
    c(224, 0, 0, 0, 4, "멀티캐스트", Special::Multicast),
    c(240, 0, 0, 0, 4, "예약", Special::Reserved),
];

/// 실무에서 흔한 사설 서브넷 (가중치 = 생성 비율)
pub const COMMON_PRIVATE: &[(Cidr, u32)] = &[
    (c(192, 168, 0, 0, 24, "공유기 기본", Special::Private), 14),
    (c(192, 168, 1, 0, 24, "공유기 기본", Special::Private), 14),
    (c(192, 168, 10, 0, 24, "사내망", Special::Private), 5),
    (c(192, 168, 100, 0, 24, "사내망", Special::Private), 4),
    (c(192, 168, 0, 0, 16, "사설 C", Special::Private), 8),
    (c(10, 0, 0, 0, 16, "사내망", Special::Private), 8),
    (c(10, 10, 0, 0, 16, "사내망", Special::Private), 4),
    (c(10, 1, 0, 0, 16, "사내망", Special::Private), 3),
    (c(10, 0, 0, 0, 8, "사설 A", Special::Private), 10),
    (c(172, 16, 0, 0, 16, "사내망", Special::Private), 4),
    (c(172, 17, 0, 0, 16, "Docker", Special::Private), 3),
    (c(172, 31, 0, 0, 16, "AWS 기본 VPC", Special::Private), 3),
    (c(172, 16, 0, 0, 12, "사설 B", Special::Private), 4),
];

fn to_u32(o: &[u32; 4]) -> u32 {
    (o[0] << 24) | (o[1] << 16) | (o[2] << 8) | o[3]
}

pub fn in_cidr(o: &[u32; 4], cidr: &Cidr) -> bool {
    let mask = if cidr.len == 0 { 0 } else { u32::MAX << (32 - cidr.len) };
    to_u32(o) & mask == to_u32(&cidr.net) & mask
}

/// 특수 대역 분류 (가장 구체적인 것)
pub fn special(o: &[u32; 4]) -> Option<&'static Cidr> {
    SPECIAL.iter().filter(|c| in_cidr(o, c)).max_by_key(|c| c.len)
}

/// IP 사전 조회 결과
#[derive(Clone, Copy, Debug)]
pub struct IpInfo {
    pub private: bool,
    pub common_private: bool,
    /// 루프백·링크로컬·문서용·벤치마크·멀티캐스트·예약 (사람이 쓰는 호스트 주소가 아님)
    pub non_host: bool,
    pub documentation: bool,
    pub registry: Registry,
    pub kr: bool,
    pub cloud: bool,
    pub well_known: bool,
}

pub fn info(o: &[u32; 4]) -> IpInfo {
    let sp = special(o).map(|c| c.kind);
    IpInfo {
        private: matches!(sp, Some(Special::Private | Special::Shared)),
        common_private: COMMON_PRIVATE.iter().filter(|(c, _)| c.len >= 16).any(|(c, _)| in_cidr(o, c)),
        non_host: matches!(
            sp,
            Some(Special::Loopback | Special::LinkLocal | Special::Documentation | Special::Benchmark | Special::Multicast | Special::Reserved)
        ),
        documentation: sp == Some(Special::Documentation),
        registry: registry(o[0]),
        kr: KR_HEAVY.contains(&o[0]),
        cloud: CLOUD.contains(&o[0]),
        well_known: WELL_KNOWN.contains(o),
    }
}

/// 학습용: 실제 환경에서 나올 법한 IP 하나 생성
pub fn sample_real(r: &mut super::synth::Rng) -> [u32; 4] {
    let host = |r: &mut super::synth::Rng| -> u32 {
        // 호스트 번호는 작은 수(게이트웨이 1, 서버 2~50)가 흔하고, 나머지는 고르게
        match r.below(10) {
            0..=1 => *[1u32, 254, 2, 10, 100, 11, 20, 5].get(r.below(8)).unwrap(),
            2..=4 => r.range(2, 60),
            _ => r.range(1, 254),
        }
    };
    let in_block = |r: &mut super::synth::Rng, cidr: &Cidr| -> [u32; 4] {
        let base = to_u32(&cidr.net);
        let span = if cidr.len >= 32 { 0 } else { (1u64 << (32 - cidr.len)) - 1 } as u32;
        let v = base | (r.next() as u32 & span);
        let mut o = [v >> 24, (v >> 16) & 255, (v >> 8) & 255, v & 255];
        o[3] = host(r);
        o
    };
    match r.below(100) {
        0..=44 => {
            let total: u32 = COMMON_PRIVATE.iter().map(|x| x.1).sum();
            let mut k = r.range(0, total - 1);
            for (cidr, w) in COMMON_PRIVATE {
                if k < *w {
                    return in_block(r, cidr);
                }
                k -= w;
            }
            unreachable!()
        }
        45..=69 => {
            let o1 = KR_HEAVY[r.below(KR_HEAVY.len())];
            [o1, r.range(0, 255), r.range(0, 255), host(r)]
        }
        70..=79 => {
            let o1 = CLOUD[r.below(CLOUD.len())];
            [o1, r.range(0, 255), r.range(0, 255), host(r)]
        }
        80..=84 => WELL_KNOWN[r.below(WELL_KNOWN.len())],
        _ => loop {
            // 그 밖의 할당된 공인 대역
            let o1 = r.range(1, 223);
            let reg = registry(o1);
            if !matches!(reg, Registry::Reserved | Registry::Private | Registry::Loopback | Registry::Multicast) {
                let o = [o1, r.range(0, 255), r.range(0, 255), host(r)];
                if !info(&o).non_host && !info(&o).private {
                    break o;
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        assert!(info(&[192, 168, 0, 1]).private && info(&[192, 168, 0, 1]).common_private);
        assert!(info(&[172, 20, 1, 1]).private);
        assert!(!info(&[172, 32, 1, 1]).private);
        assert!(info(&[192, 0, 2, 10]).documentation);
        assert!(info(&[127, 0, 0, 1]).non_host);
        assert!(info(&[168, 126, 63, 1]).well_known);
        assert_eq!(registry(211), Registry::Apnic);
        assert!(info(&[211, 234, 10, 5]).kr);
    }
}

//! 인간의 경험적 직관 — "이 숫자 배열이 실제 IP 로 보이는가".
//!
//! 사람은 `12-34-56-78`, `10-20-30-40`, `1-2-3-4`, `11-22-33-44`, `123-45-6-78` 을 보면
//! 실제 장비 주소가 아니라 **사람이 만든 배열**(연속 숫자, 등차수열, 반복, 대칭, 딱 떨어지는 수)로 느낀다.
//! 이 직관을 특징으로 만들고, IP 사전으로 만든 실제 IP 분포와 대비해 **자연스러움 모델**(1단)을 학습한다.
//! 본 모델(2단)은 이 확률을 특징으로 받는다.

use super::ipdict::{self, Registry};
use super::synth::Rng;

pub const N_NATURAL: usize = 30;

fn digits_of(o: &[u32]) -> Vec<u8> {
    o.iter().flat_map(|v| v.to_string().into_bytes()).map(|b| b - b'0').collect()
}

/// 이어 붙인 숫자에서 가장 긴 연속(±1) 구간 길이
fn longest_step_run(d: &[u8]) -> usize {
    let mut best = 1;
    for step in [1i32, -1] {
        let mut run = 1;
        for w in d.windows(2) {
            if w[1] as i32 - w[0] as i32 == step {
                run += 1;
                best = best.max(run);
            } else {
                run = 1;
            }
        }
    }
    best
}

fn is_ap(v: &[u32]) -> bool {
    v.len() >= 3 && {
        let d = v[1] as i64 - v[0] as i64;
        d != 0 && v.windows(2).all(|w| w[1] as i64 - w[0] as i64 == d)
    }
}

fn entropy(d: &[u8]) -> f32 {
    let mut c = [0f32; 10];
    d.iter().for_each(|&x| c[x as usize] += 1.0);
    let n = d.len() as f32;
    c.iter().filter(|&&k| k > 0.0).map(|&k| -(k / n) * (k / n).log2()).sum()
}

/// 4개 묶음(≤255) → 자연스러움 특징
pub fn natural_features(o: &[u32; 4]) -> [f32; N_NATURAL] {
    let mut f = [0f32; N_NATURAL];
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    let d = digits_of(o);
    let inf = ipdict::info(o);
    for i in 0..4 {
        f[i] = o[i] as f32;
    }
    f[4] = inf.registry as u32 as f32;
    f[5] = b(inf.private);
    f[6] = b(inf.common_private);
    f[7] = b(inf.non_host);
    f[8] = b(inf.kr);
    f[9] = b(inf.cloud);
    f[10] = b(inf.well_known);
    let run = longest_step_run(&d);
    f[11] = run as f32 / d.len() as f32;
    f[12] = b(run == d.len() && d.len() >= 4);
    f[13] = b(is_ap(o));
    f[14] = b(is_ap(&o[..3]) || is_ap(&o[1..]));
    f[15] = b(o.iter().all(|&x| x == o[0]));
    f[16] = o.iter().filter(|x| {
        let s = x.to_string();
        s.len() >= 2 && s.bytes().all(|c| c == s.as_bytes()[0])
    }).count() as f32
        / 4.0;
    f[17] = o.iter().filter(|&&x| x != 0 && x % 10 == 0).count() as f32 / 4.0;
    f[18] = b(o.windows(2).all(|w| w[1] > w[0]));
    f[19] = b(o.windows(2).all(|w| w[1] < w[0]));
    let mut ds = d.clone();
    ds.sort();
    ds.dedup();
    f[20] = ds.len() as f32;
    f[21] = entropy(&d);
    let lens: Vec<usize> = o.iter().map(|x| x.to_string().len()).collect();
    f[22] = b(lens.iter().all(|&l| l == lens[0]));
    f[23] = (lens[0] * 1000 + lens[1] * 100 + lens[2] * 10 + lens[3]) as f32;
    f[24] = b(o[3] == 0 || o[3] == 255);
    f[25] = *o.iter().max().unwrap() as f32;
    f[26] = *o.iter().min().unwrap() as f32;
    f[27] = b(o[0] == o[3] && o[1] == o[2]);
    f[28] = o.iter().filter(|&&x| x < 32).count() as f32;
    // 인접 묶음이 숫자 이어짐 (12|34: 2 다음 3)
    f[29] = o
        .windows(2)
        .filter(|w| {
            let a = w[0] % 10;
            let s = w[1].to_string();
            let first = (s.as_bytes()[0] - b'0') as u32;
            first == a + 1 || first + 1 == a
        })
        .count() as f32;
    f
}

// ───────── 1단 학습 데이터 ─────────

/// 사람이 만든 배열 (IP 처럼 보이지 않는 것) 생성
pub fn sample_artificial(r: &mut Rng) -> [u32; 4] {
    loop {
        let o: [u32; 4] = match r.below(9) {
            0 | 1 => {
                // 연속 숫자를 4묶음으로 자름: 12-34-56-78, 123-45-6-78, 1-23-45-67, 98-76-54-32
                let start = r.range(0, 9) as i32;
                let step = if r.chance(0.8) { 1 } else { -1 };
                let len = 4 + r.below(6);
                let s: String = (0..len).map(|i| char::from(b'0' + (start + step * i as i32).rem_euclid(10) as u8)).collect();
                let mut cuts = [0usize; 3];
                let mut ok = false;
                for _ in 0..20 {
                    let mut c = [1 + r.below(len - 3), 0, 0];
                    c[1] = c[0] + 1 + r.below(len - c[0] - 2);
                    c[2] = c[1] + 1 + r.below(len - c[1] - 1);
                    let parts = [&s[..c[0]], &s[c[0]..c[1]], &s[c[1]..c[2]], &s[c[2]..]];
                    if parts.iter().all(|p| p.len() <= 3 && p.parse::<u32>().map_or(false, |v| v <= 255)) {
                        cuts = c;
                        ok = true;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
                let p = |a: usize, z: usize| s[a..z].parse::<u32>().unwrap();
                [p(0, cuts[0]), p(cuts[0], cuts[1]), p(cuts[1], cuts[2]), p(cuts[2], len)]
            }
            2 | 3 => {
                // 등차수열: 10-20-30-40, 1-2-3-4, 11-22-33-44, 5-10-15-20, 200-150-100-50
                let d = *[1i64, 2, 5, 10, 11, 20, 22, 25, 50, 100, -1, -10, -11, -50].get(r.below(14)).unwrap();
                let a = r.range(0, 60) as i64 * if d < 0 { 5 } else { 1 };
                let v: Vec<i64> = (0..4).map(|i| a + d * i).collect();
                if v.iter().any(|&x| !(0..=255).contains(&x)) {
                    continue;
                }
                [v[0] as u32, v[1] as u32, v[2] as u32, v[3] as u32]
            }
            4 => {
                // 반복: 1-1-1-1, 11-11-11-11, 111-111-111-111, 7-7-7-7
                let x = *[1u32, 2, 3, 5, 7, 9, 11, 22, 33, 77, 99, 111, 222].get(r.below(13)).unwrap();
                [x, x, x, x]
            }
            5 => {
                // 대칭/쌍: 12-34-34-12, 1-2-2-1, 10-20-10-20
                let (a, b) = (r.range(1, 99), r.range(1, 99));
                if r.chance(0.5) {
                    [a, b, b, a]
                } else {
                    [a, b, a, b]
                }
            }
            6 => {
                // 딱 떨어지는 수: 100-200-0-0, 10-0-0-0 계열 아님 (사설 10.x 가 아닌 모양)
                let pick = |r: &mut Rng| *[0u32, 10, 20, 50, 100, 150, 200, 250].get(r.below(8)).unwrap();
                [pick(r), pick(r), pick(r), pick(r)]
            }
            7 => {
                // 시각·날짜를 4묶음으로: 12-30-45-00, 24-11-13-09, 월일 쌍 10-26-10-31
                match r.below(3) {
                    0 => [r.range(0, 23), r.range(0, 59), r.range(0, 59), r.range(0, 99)],
                    1 => [r.range(10, 30), r.range(1, 12), r.range(1, 28), r.range(0, 23)],
                    _ => [r.range(1, 12), r.range(1, 28), r.range(1, 12), r.range(1, 31)],
                }
            }
            _ => {
                // 버전·장절 번호처럼 작은 수: 1-0-3-12, 2-1-0-4
                [r.range(0, 9), r.range(0, 20), r.range(0, 30), r.range(0, 40)]
            }
        };
        return o;
    }
}

/// 1단 학습 데이터: (특징, 1=실제 IP 같음 / 0=사람이 만든 배열)
pub fn natural_dataset(seed: u64, n: usize) -> (Vec<Vec<f32>>, Vec<usize>) {
    let mut r = Rng::new(seed ^ 0x5151);
    let mut x = Vec::with_capacity(n);
    let mut y = Vec::with_capacity(n);
    for _ in 0..n {
        let (o, label) = if r.chance(0.5) { (ipdict::sample_real(&mut r), 1) } else { (sample_artificial(&mut r), 0) };
        x.push(natural_features(&o).to_vec());
        y.push(label);
    }
    (x, y)
}

/// 예약·멀티캐스트 대역 등 /8 이 사용 불가인가
pub fn unusable(o: &[u32; 4]) -> bool {
    matches!(ipdict::registry(o[0]), Registry::Reserved | Registry::Multicast | Registry::Loopback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intuition_features() {
        let f = natural_features(&[12, 34, 56, 78]);
        assert_eq!(f[12], 1.0, "12345678 연속");
        assert_eq!(f[13], 1.0, "12,34,56,78 등차");
        let f = natural_features(&[123, 45, 6, 78]);
        assert_eq!(f[12], 1.0);
        let f = natural_features(&[10, 20, 30, 40]);
        assert_eq!(f[13], 1.0);
        let f = natural_features(&[192, 168, 0, 1]);
        assert_eq!(f[12], 0.0);
        assert_eq!(f[13], 0.0);
        assert_eq!(f[6], 1.0);
    }
}

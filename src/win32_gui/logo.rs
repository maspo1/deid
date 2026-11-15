//! 정보 창의 GitHub 마크 — Octicons `mark-github` (MIT, © GitHub) SVG 경로를 DPI 크기로 직접 래스터.
//! 이미지 파일을 넣지 않으므로 어떤 배율에서도 칼같이 그려진다.

/// viewBox 0 0 16 16
const MARK: &str = "M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z";

type P = (f64, f64);

/// SVG 경로(M/C/c/A/z 만 사용) → 꺾은선 하나
fn outline() -> Vec<P> {
    let b = MARK.as_bytes();
    let mut i = 0;
    let num = |i: &mut usize| -> f64 {
        while *i < b.len() && (b[*i] == b' ' || b[*i] == b',') {
            *i += 1;
        }
        let s = *i;
        if *i < b.len() && b[*i] == b'-' {
            *i += 1;
        }
        let mut dot = false;
        while *i < b.len() && (b[*i].is_ascii_digit() || (b[*i] == b'.' && !dot)) {
            dot |= b[*i] == b'.';
            *i += 1;
        }
        MARK[s..*i].parse().unwrap_or(0.0)
    };
    let flag = |i: &mut usize| -> bool {
        while b[*i] == b' ' {
            *i += 1;
        }
        *i += 1;
        b[*i - 1] == b'1'
    };
    let (mut pts, mut cur, mut cmd) = (Vec::new(), (0.0, 0.0), b'M');
    while i < b.len() {
        if b[i].is_ascii_alphabetic() {
            cmd = b[i];
            i += 1;
            if cmd == b'z' {
                break;
            }
            continue;
        }
        match cmd {
            b'M' => {
                cur = (num(&mut i), num(&mut i));
                pts.push(cur);
            }
            b'C' | b'c' => {
                let o = if cmd == b'c' { cur } else { (0.0, 0.0) };
                let mut q = [(0.0, 0.0); 3];
                for p in &mut q {
                    *p = (o.0 + num(&mut i), o.1 + num(&mut i));
                }
                for k in 1..=8 {
                    let t = k as f64 / 8.0;
                    let u = 1.0 - t;
                    let f = |a: f64, b1: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b1 + 3.0 * u * t * t * c + t * t * t * d;
                    pts.push((f(cur.0, q[0].0, q[1].0, q[2].0), f(cur.1, q[0].1, q[1].1, q[2].1)));
                }
                cur = q[2];
            }
            b'A' => {
                // 원호 (rx = ry, 회전 0): 끝점 두 개와 반지름으로 중심을 구해 잘게 나눔
                let r = num(&mut i);
                let _ry = num(&mut i);
                let _rot = num(&mut i);
                let (large, sweep) = (flag(&mut i), flag(&mut i));
                let end = (num(&mut i), num(&mut i));
                let (mx, my) = ((cur.0 + end.0) / 2.0, (cur.1 + end.1) / 2.0);
                let (dx, dy) = (end.0 - cur.0, end.1 - cur.1);
                let d = (dx * dx + dy * dy).sqrt();
                let h = (r * r - d * d / 4.0).max(0.0).sqrt();
                let s = if large == sweep { -1.0 } else { 1.0 };
                let c = (mx - s * h * dy / d, my + s * h * dx / d);
                let a0 = (cur.1 - c.1).atan2(cur.0 - c.0);
                let mut da = (end.1 - c.1).atan2(end.0 - c.0) - a0;
                if sweep && da < 0.0 {
                    da += std::f64::consts::TAU;
                } else if !sweep && da > 0.0 {
                    da -= std::f64::consts::TAU;
                }
                for k in 1..=24 {
                    let a = a0 + da * k as f64 / 24.0;
                    pts.push((c.0 + r * a.cos(), c.1 + r * a.sin()));
                }
                cur = end;
            }
            _ => i += 1,
        }
    }
    pts
}

fn inside(poly: &[P], x: f64, y: f64) -> bool {
    let mut odd = false;
    let mut j = poly.len() - 1;
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[j]);
        if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) {
            odd = !odd;
        }
        j = k;
    }
    odd
}

/// `size`×`size` 화소, 배경색 `bg`(COLORREF 0x00BBGGRR) 위에 4×4 표본으로 부드럽게 칠한 불투명 BGRA
pub fn github_mark(size: i32, bg: u32) -> Vec<u32> {
    let poly = outline();
    let ink = (0x24u32, 0x29u32, 0x2Fu32); // GitHub 기본 글자색 #24292f
    let (br, bgc, bb) = (bg & 0xFF, (bg >> 8) & 0xFF, (bg >> 16) & 0xFF);
    let n = size.max(1) as usize;
    let scale = 16.0 / n as f64;
    let mut out = vec![0u32; n * n];
    for y in 0..n {
        for x in 0..n {
            let mut hit = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let (fx, fy) = ((x as f64 + (sx as f64 + 0.5) / 4.0) * scale, (y as f64 + (sy as f64 + 0.5) / 4.0) * scale);
                    hit += inside(&poly, fx, fy) as u32;
                }
            }
            let mix = |i: u32, b: u32| (i * hit + b * (16 - hit)) / 16;
            out[y * n + x] = 0xFF00_0000 | mix(ink.0, br) << 16 | mix(ink.1, bgc) << 8 | mix(ink.2, bb);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_shape() {
        let n = 32;
        let px = github_mark(n, 0x00FF_FFFF);
        let at = |x: i32, y: i32| px[(y * n + x) as usize] & 0xFF_FFFF;
        assert_eq!(at(0, 0), 0xFF_FFFF, "모서리는 원 바깥 → 배경");
        assert_eq!(at(3, 16), 0x24_292F, "원 왼쪽 → 진한 색");
        assert_eq!(at(16, 14), 0xFF_FFFF, "고양이 얼굴은 비어 있음");
        assert_eq!(at(28, 16), 0x24_292F, "원호(A 명령)로 그린 오른쪽 → 진한 색");
    }
}

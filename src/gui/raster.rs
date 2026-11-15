//! CPU 렌더러 — egui 가 만든 삼각형 메시를 CPU 로 그린다.
//!
//! GPU(OpenGL/DirectX) 를 쓰면 그래픽 드라이버(셰이더 컴파일러 등)가 프로세스에 수십 MB 를 올린다.
//! 이 앱의 화면은 사각형·글자 위주라 CPU 로 그려도 충분히 빠르고, 메모리는 창 크기만큼의 버퍼면 된다.
//!
//! * 삼각형: 경계 상자 + 에지 함수, 공유 변 중복 방지(top-left 규칙)
//! * 색: egui 의 사전 곱셈(premultiplied) sRGB 그대로 혼합 — `out = src + dst × (1 − αsrc)`
//! * 텍스처: 글꼴 아틀라스는 1바이트(덮임 정도)로 저장해 메모리 1/4

use egui::epaint::{ClippedPrimitive, ImageData, ImageDelta, Primitive, Vertex};
use egui::{Color32, TextureId, TexturesDelta};
use std::collections::HashMap;

enum Pixels {
    /// 글꼴 아틀라스: 흰색 + 알파 → 1바이트
    Alpha(Vec<u8>),
    Rgba(Vec<Color32>),
}

struct Texture {
    w: usize,
    h: usize,
    px: Pixels,
}

impl Texture {
    /// 최근접 샘플링 (글꼴은 실제 화소 크기로 래스터되어 있으므로 충분)
    #[inline]
    fn sample(&self, u: f32, v: f32) -> [u32; 4] {
        let x = ((u * self.w as f32) as isize).clamp(0, self.w as isize - 1) as usize;
        let y = ((v * self.h as f32) as isize).clamp(0, self.h as isize - 1) as usize;
        let i = y * self.w + x;
        match &self.px {
            Pixels::Alpha(a) => {
                let c = a[i] as u32;
                [c, c, c, c]
            }
            Pixels::Rgba(p) => {
                let c = p[i];
                [c.r() as u32, c.g() as u32, c.b() as u32, c.a() as u32]
            }
        }
    }
}

fn to_pixels(img: &ImageData) -> (usize, usize, Pixels) {
    match img {
        ImageData::Color(c) => (c.size[0], c.size[1], Pixels::Rgba(c.pixels.clone())),
        ImageData::Font(f) => (f.size[0], f.size[1], Pixels::Alpha(f.srgba_pixels(None).map(|c| c.a()).collect())),
    }
}

#[derive(Default)]
pub struct Raster {
    textures: HashMap<TextureId, Texture>,
}

impl Raster {
    pub fn set_textures(&mut self, delta: &TexturesDelta) {
        for (id, d) in &delta.set {
            self.update(*id, d);
        }
    }

    pub fn free_textures(&mut self, delta: &TexturesDelta) {
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    fn update(&mut self, id: TextureId, d: &ImageDelta) {
        let (w, h, px) = to_pixels(&d.image);
        let Some([x0, y0]) = d.pos else {
            self.textures.insert(id, Texture { w, h, px });
            return;
        };
        let Some(t) = self.textures.get_mut(&id) else { return };
        // 부분 갱신 (글꼴 아틀라스에 새 글자 추가)
        for row in 0..h {
            let (ty, src) = (y0 + row, row * w);
            if ty >= t.h {
                break;
            }
            let n = w.min(t.w.saturating_sub(x0));
            let dst = ty * t.w + x0;
            match (&mut t.px, &px) {
                (Pixels::Alpha(d), Pixels::Alpha(s)) => d[dst..dst + n].copy_from_slice(&s[src..src + n]),
                (Pixels::Rgba(d), Pixels::Rgba(s)) => d[dst..dst + n].copy_from_slice(&s[src..src + n]),
                (Pixels::Rgba(d), Pixels::Alpha(s)) => {
                    for i in 0..n {
                        let a = s[src + i];
                        d[dst + i] = Color32::from_rgba_premultiplied(a, a, a, a);
                    }
                }
                (Pixels::Alpha(d), Pixels::Rgba(s)) => {
                    for i in 0..n {
                        d[dst + i] = s[src + i].a();
                    }
                }
            }
        }
    }

    /// `buf` (0x00RRGGBB, 너비 `w`) 에 그리기
    pub fn render(&self, buf: &mut [u32], w: usize, h: usize, prims: &[ClippedPrimitive], ppp: f32, clear: Color32) {
        let bg = (clear.r() as u32) << 16 | (clear.g() as u32) << 8 | clear.b() as u32;
        buf.fill(bg);
        for p in prims {
            let Primitive::Mesh(mesh) = &p.primitive else { continue };
            let c = p.clip_rect;
            let clip = [
                ((c.min.x * ppp).round().max(0.0) as i32).min(w as i32),
                ((c.min.y * ppp).round().max(0.0) as i32).min(h as i32),
                ((c.max.x * ppp).round().max(0.0) as i32).min(w as i32),
                ((c.max.y * ppp).round().max(0.0) as i32).min(h as i32),
            ];
            if clip[0] >= clip[2] || clip[1] >= clip[3] {
                continue;
            }
            let tex = self.textures.get(&mesh.texture_id);
            for tri in mesh.indices.chunks_exact(3) {
                let v = |i: u32| &mesh.vertices[i as usize];
                triangle(buf, w, clip, tex, ppp, [v(tri[0]), v(tri[1]), v(tri[2])]);
            }
        }
    }
}

#[inline]
fn mul(a: u32, b: u32) -> u32 {
    (a * b + 127) / 255
}

#[inline]
fn blend(dst: &mut u32, s: [u32; 4]) {
    let [r, g, b, a] = s;
    if a == 0 && r == 0 && g == 0 && b == 0 {
        return;
    }
    if a >= 255 {
        *dst = r.min(255) << 16 | g.min(255) << 8 | b.min(255);
        return;
    }
    let ia = 255 - a;
    let d = *dst;
    let nr = (r + mul((d >> 16) & 255, ia)).min(255);
    let ng = (g + mul((d >> 8) & 255, ia)).min(255);
    let nb = (b + mul(d & 255, ia)).min(255);
    *dst = nr << 16 | ng << 8 | nb;
}

#[inline]
fn shade(color: [f32; 4], tex: Option<&Texture>, u: f32, v: f32) -> [u32; 4] {
    let c = color.map(|x| x.clamp(0.0, 255.0) as u32);
    match tex {
        Some(t) => {
            let s = t.sample(u, v);
            [mul(c[0], s[0]), mul(c[1], s[1]), mul(c[2], s[2]), mul(c[3], s[3])]
        }
        None => c,
    }
}

fn rgba(v: &Vertex) -> [f32; 4] {
    [v.color.r() as f32, v.color.g() as f32, v.color.b() as f32, v.color.a() as f32]
}

fn triangle(buf: &mut [u32], w: usize, clip: [i32; 4], tex: Option<&Texture>, ppp: f32, mut v: [&Vertex; 3]) {
    let pos = |v: &Vertex| (v.pos.x * ppp, v.pos.y * ppp);
    let mut p = [pos(v[0]), pos(v[1]), pos(v[2])];
    let edge = |a: (f32, f32), b: (f32, f32), c: (f32, f32)| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    let mut area = edge(p[0], p[1], p[2]);
    if area.abs() < 1e-6 {
        return;
    }
    if area < 0.0 {
        v.swap(1, 2);
        p.swap(1, 2);
        area = -area;
    }
    let min_x = (p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor() as i32).max(clip[0]);
    let max_x = (p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil() as i32).min(clip[2]);
    let min_y = (p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor() as i32).max(clip[1]);
    let max_y = (p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil() as i32).min(clip[3]);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    // 에지 i 는 꼭짓점 i 맞은편 변: e0 = v1→v2, e1 = v2→v0, e2 = v0→v1
    // e(x, y) = A·x + B·y + C, 내부는 모두 ≥ 0 (공유 변은 top-left 규칙으로 한쪽만)
    let setup = |a: (f32, f32), b: (f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let tl = dy > 0.0 || (dy == 0.0 && dx < 0.0);
        (-dy, dx, dy * a.0 - dx * a.1, tl)
    };
    let e = [setup(p[1], p[2]), setup(p[2], p[0]), setup(p[0], p[1])];
    let inside = |w: f32, tl: bool| w > 0.0 || (w == 0.0 && tl);

    let c = [rgba(v[0]), rgba(v[1]), rgba(v[2])];
    let uv = [v[0].uv, v[1].uv, v[2].uv];
    // 대부분의 삼각형(사각형 배경)은 색·텍스처 좌표가 일정 → 한 번만 계산
    let flat = v[0].color == v[1].color && v[1].color == v[2].color && uv[0] == uv[1] && uv[1] == uv[2];
    let flat_px = flat.then(|| shade(c[0], tex, uv[0].x, uv[0].y));
    let inv = 1.0 / area;

    let x0 = min_x as f32 + 0.5;
    for y in min_y..max_y {
        let fy = y as f32 + 0.5;
        let mut w0 = e[0].0 * x0 + e[0].1 * fy + e[0].2;
        let mut w1 = e[1].0 * x0 + e[1].1 * fy + e[1].2;
        let mut w2 = e[2].0 * x0 + e[2].1 * fy + e[2].2;
        let row = y as usize * w;
        for x in min_x..max_x {
            if inside(w0, e[0].3) && inside(w1, e[1].3) && inside(w2, e[2].3) {
                let px = match flat_px {
                    Some(px) => px,
                    None => {
                        let (l0, l1, l2) = (w0 * inv, w1 * inv, w2 * inv);
                        let col = [0, 1, 2, 3].map(|k| c[0][k] * l0 + c[1][k] * l1 + c[2][k] * l2);
                        let u = uv[0].x * l0 + uv[1].x * l1 + uv[2].x * l2;
                        let vv = uv[0].y * l0 + uv[1].y * l1 + uv[2].y * l2;
                        shade(col, tex, u, vv)
                    }
                };
                blend(&mut buf[row + x as usize], px);
            }
            w0 += e[0].0;
            w1 += e[1].0;
            w2 += e[2].0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::{Mesh, WHITE_UV};
    use egui::{pos2, Rect};

    fn quad_mesh(r: Rect, color: Color32) -> ClippedPrimitive {
        let mut m = Mesh::default();
        m.add_colored_rect(r, color);
        ClippedPrimitive { clip_rect: Rect::EVERYTHING, primitive: Primitive::Mesh(m) }
    }

    #[test]
    fn adjacent_triangles_cover_once() {
        // 반투명 사각형(삼각형 2개)의 대각선이 두 번 칠해지면 진해진다 → 모든 화소가 같은 색이어야 함
        let raster = Raster::default();
        let (w, h) = (16, 16);
        let mut buf = vec![0u32; w * h];
        let half = Color32::from_rgba_premultiplied(128, 0, 0, 128);
        raster.render(&mut buf, w, h, &[quad_mesh(Rect::from_min_max(pos2(0.0, 0.0), pos2(16.0, 16.0)), half)], 1.0, Color32::BLACK);
        assert!(buf.iter().all(|&p| p == buf[0]), "대각선 이음새");
        assert_eq!(buf[0] >> 16, 128);
        let _ = WHITE_UV;
    }

    #[test]
    fn clip_rect_and_scale() {
        let raster = Raster::default();
        let (w, h) = (20, 20);
        let mut buf = vec![0u32; w * h];
        let mut p = quad_mesh(Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0)), Color32::WHITE);
        p.clip_rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(5.0, 10.0));
        raster.render(&mut buf, w, h, &[p], 2.0, Color32::BLACK);
        assert_eq!(buf[9], 0xFFFFFF); // x=9 < 5*2
        assert_eq!(buf[10], 0); // 잘림
        assert_eq!(buf[19 * w], 0xFFFFFF); // y=19 < 10*2
    }
}

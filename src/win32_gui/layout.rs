//! 배치 계산 (순수 함수) — 창 크기·DPI 를 받아 컨트롤 사각형을 돌려준다. Win32 호출 없음 → 단위 테스트.
//! 모든 상수는 96 DPI 기준이고 `px` 로 현재 DPI 에 맞춘다.

use super::dpi::px;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct R {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl R {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> R {
        R { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// 96 DPI 기준 치수
const M: i32 = 8; // 바깥 여백
const G: i32 = 6; // 컨트롤 간격
const ROW: i32 = 24; // 입력칸·버튼 높이
const LABEL_W: i32 = 64;
const BTN_W: i32 = 92;
pub const PANEL_W: i32 = 300;

pub struct Main {
    pub in_label: R,
    pub in_edit: R,
    pub in_btn: R,
    pub out_label: R,
    pub out_edit: R,
    pub out_btn: R,
    pub panel: R,
    pub tab: R,
    pub progress: R,
    pub btn_discard: R,
    pub btn_report: R,
    pub btn_out: R,
    pub btn_start: R,
}

/// 메인 창. `status_h` 는 상태 표시줄의 실제 높이(화소).
pub fn main(w: i32, h: i32, status_h: i32, dpi: u32) -> Main {
    let p = |v| px(v, dpi);
    let (m, g, row) = (p(M), p(G), p(ROW));
    let label_w = p(LABEL_W);
    let btn_w = p(BTN_W);
    let row1 = m;
    let row2 = row1 + row + g;
    let edit_x = m + label_w + g;
    let edit_w = (w - edit_x - g - btn_w - m).max(p(80));
    let top_h = row2 + row + g;

    let start_h = p(30);
    let bottom_btn_y = h - status_h - m - start_h;
    let progress_y = bottom_btn_y - g - p(18);
    let body_h = (progress_y - g - top_h).max(p(60));

    let panel = R::new(m, top_h, p(PANEL_W), body_h);
    let tab_x = panel.right() + g;
    Main {
        in_label: R::new(m, row1 + p(4), label_w, row - p(4)),
        in_edit: R::new(edit_x, row1, edit_w, row),
        in_btn: R::new(edit_x + edit_w + g, row1, btn_w, row),
        out_label: R::new(m, row2 + p(4), label_w, row - p(4)),
        out_edit: R::new(edit_x, row2, edit_w, row),
        out_btn: R::new(edit_x + edit_w + g, row2, btn_w, row),
        panel,
        tab: R::new(tab_x, top_h, (w - tab_x - m).max(p(100)), body_h),
        progress: R::new(m, progress_y, w - 2 * m, p(18)),
        btn_start: R::new(w - m - p(120), bottom_btn_y, p(120), start_h),
        btn_out: R::new(w - m - p(120) - g - p(110), bottom_btn_y, p(110), start_h),
        btn_report: R::new(w - m - p(120) - 2 * g - p(220), bottom_btn_y, p(110), start_h),
        btn_discard: R::new(w - m - p(120) - 3 * g - p(330), bottom_btn_y, p(110), start_h),
    }
}

/// 창 최소 크기 (96 DPI)
pub const MIN_W: i32 = 820;
pub const MIN_H: i32 = 560;

// ───────────────────────────── 탭 페이지 ─────────────────────────────

pub struct Results {
    pub summary: R,
    pub review_only: R,
    pub kind: R,
    pub search: R,
    pub note: R,
    pub list: R,
}

pub fn results(pg: R, dpi: u32) -> Results {
    let p = |v| px(v, dpi);
    let y1 = pg.y + p(2);
    let y2 = y1 + p(20) + p(4);
    let y3 = y2 + p(ROW) + p(4);
    let list_y = y3 + p(18) + p(4);
    Results {
        summary: R::new(pg.x, y1, pg.w, p(20)),
        review_only: R::new(pg.x, y2, p(110), p(ROW)),
        kind: R::new(pg.x + p(116), y2, p(150), p(300)), // 콤보 높이 = 펼친 목록 포함
        search: R::new(pg.x + p(272), y2, p(220).min(pg.w - p(272)).max(p(60)), p(ROW)),
        note: R::new(pg.x, y3, pg.w, p(18)),
        list: R::new(pg.x, list_y, pg.w, pg.bottom() - list_y),
    }
}

pub struct Forensic {
    pub expand: R,
    pub collapse: R,
    pub only_found: R,
    pub search: R,
    pub legend: R,
    pub tree: R,
    /// 위아래 크기 조절 막대 (마우스로 끌기)
    pub splitter: R,
    pub title: R,
    pub tab_recs: R,
    pub tab_props: R,
    pub detail: R,
    pub btn_src: R,
    pub btn_dst: R,
}

/// `split` = 아래 상세 창 높이(화소). 너무 크거나 작으면 줄인다.
pub fn forensic(pg: R, split: i32, dpi: u32) -> Forensic {
    let p = |v| px(v, dpi);
    let tb_y = pg.y + p(2);
    let tree_y = tb_y + p(ROW) + p(4);
    let split = split.clamp(p(110), (pg.bottom() - tree_y - p(80)).max(p(110)));
    let det_y = pg.bottom() - split;
    let sp = R::new(pg.x, det_y - p(6), pg.w, p(6));
    let title_y = det_y;
    let list_y = title_y + p(ROW) + p(4);
    let btn_y = pg.bottom() - p(ROW);
    Forensic {
        expand: R::new(pg.x, tb_y, p(80), p(ROW)),
        collapse: R::new(pg.x + p(84), tb_y, p(80), p(ROW)),
        only_found: R::new(pg.x + p(172), tb_y, p(140), p(ROW)),
        search: R::new(pg.x + p(316), tb_y, p(170), p(ROW)),
        legend: R::new(pg.x + p(494), tb_y + p(4), (pg.w - p(494)).max(0), p(ROW) - p(4)),
        tree: R::new(pg.x, tree_y, pg.w, sp.y - tree_y),
        splitter: sp,
        title: R::new(pg.x, title_y + p(4), (pg.w - p(260)).max(p(60)), p(ROW) - p(4)),
        tab_recs: R::new(pg.right() - p(254), title_y, p(130), p(ROW)),
        tab_props: R::new(pg.right() - p(120), title_y, p(120), p(ROW)),
        detail: R::new(pg.x, list_y, pg.w, btn_y - p(4) - list_y),
        btn_src: R::new(pg.x, btn_y, p(110), p(ROW)),
        btn_dst: R::new(pg.x + p(116), btn_y, p(110), p(ROW)),
    }
}

pub struct Preview {
    pub help: R,
    pub input: R,
    pub out_label: R,
    pub output: R,
    pub count: R,
    pub list: R,
}

pub fn preview(pg: R, dpi: u32) -> Preview {
    let p = |v| px(v, dpi);
    let y1 = pg.y + p(2);
    let in_y = y1 + p(20);
    let ol_y = in_y + p(110) + p(6);
    let out_y = ol_y + p(20);
    let c_y = out_y + p(80) + p(6);
    let l_y = c_y + p(20);
    Preview {
        help: R::new(pg.x, y1, pg.w, p(18)),
        input: R::new(pg.x, in_y, pg.w, p(110)),
        out_label: R::new(pg.x, ol_y, pg.w, p(18)),
        output: R::new(pg.x, out_y, pg.w, p(80)),
        count: R::new(pg.x, c_y, pg.w, p(18)),
        list: R::new(pg.x, l_y, pg.w, (pg.bottom() - l_y).max(p(40))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(r: R, outer: R) -> bool {
        r.x >= outer.x && r.y >= outer.y && r.right() <= outer.right() && r.bottom() <= outer.bottom()
    }

    fn overlap(a: R, b: R) -> bool {
        a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
    }

    /// 최소 크기·여러 배율에서 컨트롤이 창 밖으로 나가거나 서로 겹치지 않는다
    #[test]
    fn main_layout_fits_at_all_scales() {
        for dpi in [96u32, 120, 144, 192] {
            let (w, h) = (px(MIN_W, dpi), px(MIN_H, dpi));
            let sb = px(22, dpi);
            let m = main(w, h, sb, dpi);
            let client = R::new(0, 0, w, h - sb);
            let all = [m.in_label, m.in_edit, m.in_btn, m.out_label, m.out_edit, m.out_btn, m.panel, m.tab, m.progress, m.btn_discard, m.btn_report, m.btn_out, m.btn_start];
            for (i, a) in all.iter().enumerate() {
                assert!(inside(*a, client), "dpi {dpi}: {i}번 {a:?} 가 창 밖");
                for b in &all[i + 1..] {
                    assert!(!overlap(*a, *b), "dpi {dpi}: {a:?} 와 {b:?} 겹침");
                }
            }
        }
    }

    #[test]
    fn forensic_split_is_clamped_and_stacked() {
        let dpi = 144;
        let pg = R::new(0, 0, px(500, dpi), px(400, dpi));
        for split in [0, 50, 200, 10_000] {
            let f = forensic(pg, px(split, dpi), dpi);
            assert!(f.tree.h > 0 && f.detail.h > 0, "split {split}: {:?} {:?}", f.tree, f.detail);
            assert!(f.tree.bottom() <= f.splitter.y && f.splitter.bottom() <= f.title.y);
            assert!(inside(f.btn_dst, pg) && inside(f.detail, pg));
        }
    }
}

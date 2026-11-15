//! 검출 결과 탭 — 보고서(CSV)와 같은 4열 가상 리스트뷰 + 필터(검토필요만·타입·검색) + 우클릭 메뉴.
//! 행마다 체크 상자: 해제하면 그 값(같은 값 모두)은 치환하지 않는다 (`exclude`).
//! 같은 값·같은 정보타입은 한 행으로 묶고 [+] 로 펼친다 (묶음을 해제하면 전부 함께 해제).

use super::listview::{self, col, Col};
use super::exclude::{self, Check};
use super::{ctl, id, layout, rgb, Act, App};
use crate::gui::scan::{Scan, Src};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const COLS: [Col; 4] = [col("검출된정보", 180), col("치환정보", 180), col("정보타입", 140), col("파일경로", 0)];

#[derive(Default)]
pub struct Page {
    summary: HWND,
    review: HWND,
    kind: HWND,
    search: HWND,
    pub note: HWND,
    pub(super) list: HWND,
    /// 필터에 맞는 검출을 (원본 값, 정보타입) 으로 묶은 것 — 데이터는 Scan 에만 있고 여기엔 (위치, 번호) 만
    groups: Vec<Vec<(Src, usize)>>,
    /// 펼친 묶음 (원본 값, 정보타입) — 필터를 바꿔도 유지
    open: std::collections::HashSet<(String, String)>,
    /// 보이는 행
    rows: Vec<Row>,
    kinds: Vec<String>,
    key: Option<(bool, Option<String>, String, u64)>,
}

/// 보이는 행: 묶음(1건이면 그냥 한 행) 또는 펼친 묶음 안의 1건
#[derive(Clone, Copy, PartialEq, Debug)]
enum Row {
    Group(u32),
    Member(u32, u32),
}

/// 같은 (원본 값, 정보타입) 끼리 묶음 — 처음 나온 순서 유지
fn group(scan: &Scan, hits: &[(Src, usize)]) -> Vec<Vec<(Src, usize)>> {
    let mut at: std::collections::HashMap<(&str, &str), usize> = Default::default();
    let mut out: Vec<Vec<(Src, usize)>> = Vec::new();
    for &(s, k) in hits {
        let r = &scan.records(s)[k];
        let i = *at.entry((r.original.as_str(), r.label.as_str())).or_insert_with(|| {
            out.push(Vec::new());
            out.len() - 1
        });
        out[i].push((s, k));
    }
    out
}

fn group_key(scan: &Scan, g: &[(Src, usize)]) -> (String, String) {
    let r = &scan.records(g[0].0)[g[0].1];
    (r.original.clone(), r.label.clone())
}

/// 펼친 묶음만 하위 행을 보인다
fn visible(scan: &Scan, groups: &[Vec<(Src, usize)>], open: &std::collections::HashSet<(String, String)>) -> Vec<Row> {
    let mut v = Vec::with_capacity(groups.len());
    for (i, g) in groups.iter().enumerate() {
        v.push(Row::Group(i as u32));
        if g.len() > 1 && open.contains(&group_key(scan, g)) {
            v.extend((0..g.len()).map(|m| Row::Member(i as u32, m as u32)));
        }
    }
    v
}

/// 타입별 건수 (많은 순)
fn counts(scan: &Scan) -> Vec<(String, usize)> {
    let mut m: std::collections::BTreeMap<&str, usize> = Default::default();
    for r in scan.all_records() {
        *m.entry(r.base()).or_default() += 1;
    }
    let mut v: Vec<_> = m.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

/// 필터에 맞는 행 (파일명 검출 먼저, 그다음 파일 순)
fn filter(scan: &Scan, review_only: bool, kind: Option<&str>, q: &str) -> Vec<(Src, usize)> {
    let srcs = (0..scan.names.len()).map(Src::Name).chain((0..scan.entries.len()).map(Src::File));
    let mut out = Vec::new();
    for s in srcs {
        let rel = scan.rel(scan.path(s)).to_lowercase();
        for (k, r) in scan.records(s).iter().enumerate() {
            if (!review_only || r.review) && kind.map_or(true, |t| r.base() == t) && (q.is_empty() || r.original.to_lowercase().contains(q) || rel.contains(q)) {
                out.push((s, k));
            }
        }
    }
    out
}

impl Page {
    /// 다른 검사 결과로 바뀌면 필터 캐시를 버림
    pub fn reset(&mut self) {
        self.key = None;
    }
}

impl App {
    pub(super) fn results_create(&mut self) {
        let (h, dpi) = (self.hwnd, self.dpi());
        let reg = &mut self.reg;
        let p = &mut self.res;
        p.summary = ctl::label(h, "", reg, super::FontKind::Bold);
        p.review = ctl::checkbox(h, "검토필요만", id::R_REVIEW, reg);
        p.kind = ctl::child(h, ctl::COMBOBOX, "", WS_TABSTOP.0 | WS_VSCROLL.0 | 3 /* CBS_DROPDOWNLIST */, 0, id::R_KIND, reg, super::FontKind::Ui);
        p.search = ctl::edit(h, id::R_SEARCH, 0, reg, super::FontKind::Ui);
        ctl::cue(p.search, "검출 정보·경로 검색");
        p.note = ctl::label(h, "※ 주황 글자(검토필요) 행을 확인하세요. 치환하지 않을 값은 체크를 해제하거나 우클릭 → 제외 (같은 값은 함께 제외).", reg, super::FontKind::Ui);
        p.list = listview::create(h, id::R_LIST, true, 0, &COLS, dpi, reg, super::FontKind::Ui);
        listview::multi_select(p.list);
        self.results_fill_kinds(&[]);
    }

    pub(super) fn results_layout(&self, pg: layout::R) {
        let l = layout::results(pg, self.dpi());
        let p = &self.res;
        for (h, r) in [(p.summary, l.summary), (p.review, l.review_only), (p.kind, l.kind), (p.search, l.search), (p.note, l.note), (p.list, l.list)] {
            ctl::place(h, r);
        }
        listview::fit_columns(p.list, &COLS, l.list.w, self.dpi());
    }

    pub(super) fn results_columns(&self) {
        self.results_layout(self.page());
    }

    pub(super) fn results_show(&self, on: bool) {
        let p = &self.res;
        for h in [p.summary, p.review, p.kind, p.search, p.list] {
            ctl::show(h, on);
        }
        let review = self.scan.as_ref().is_some_and(|s| s.all_records().any(|r| r.review));
        ctl::show(p.note, on && review);
    }

    fn results_fill_kinds(&mut self, kinds: &[String]) {
        let cur = self.results_kind();
        let c = self.res.kind;
        ctl::send(c, CB_RESETCONTENT, 0, 0);
        for t in std::iter::once("모든 타입").chain(kinds.iter().map(String::as_str)) {
            let w = ctl::wide(t);
            ctl::send(c, CB_ADDSTRING, 0, w.as_ptr() as isize);
        }
        let sel = cur.and_then(|k| kinds.iter().position(|x| *x == k)).map_or(0, |i| i + 1);
        ctl::send(c, CB_SETCURSEL, sel, 0);
        self.res.kinds = kinds.to_vec();
    }

    fn results_kind(&self) -> Option<String> {
        let i = ctl::send(self.res.kind, CB_GETCURSEL, 0, 0);
        (i > 0).then(|| self.res.kinds.get(i as usize - 1).cloned()).flatten()
    }

    pub(super) fn results_refresh(&mut self) {
        let Some(scan) = &self.scan else {
            ctl::set_text(self.res.summary, "검사를 마치면 검출된 정보가 여기에 표시됩니다. 보고서(CSV)와 같은 내용입니다.");
            listview::set_count(self.res.list, 0);
            return;
        };
        let kinds = counts(scan);
        let total: usize = kinds.iter().map(|k| k.1).sum();
        let review = scan.all_records().filter(|r| r.review).count();
        let mut sum = format!("총 {total}건");
        if review > 0 {
            sum.push_str(&format!("   검토필요 {review}건"));
        }
        let ex = exclude::count(scan, &self.excluded);
        if ex > 0 {
            sum.push_str(&format!("   제외 {ex}건"));
        }
        for (k, n) in &kinds {
            sum.push_str(&format!("  ·  {k} {n}"));
        }
        ctl::set_text(self.res.summary, &sum);
        let names: Vec<String> = kinds.into_iter().map(|k| k.0).collect();
        if names != self.res.kinds {
            self.results_fill_kinds(&names);
        }
        ctl::show(self.res.note, self.tab == super::T_RESULTS && review > 0);

        let key = (ctl::checked(self.res.review), self.results_kind(), ctl::text(self.res.search).trim().to_lowercase(), self.scan.as_ref().map_or(0, |s| s.version));
        if self.res.key.as_ref() != Some(&key) {
            let scan = self.scan.as_ref().unwrap();
            self.res.groups = group(scan, &filter(scan, key.0, key.1.as_deref(), &key.2));
            self.res.key = Some(key);
            self.results_rows();
        }
    }

    pub(super) fn results_command(&mut self, cid: u16, code: u32) {
        match (cid, code) {
            (id::R_REVIEW, BN_CLICKED) | (id::R_KIND, CBN_SELCHANGE) | (id::R_SEARCH, EN_CHANGE) => self.results_refresh(),
            _ => {}
        }
    }

    /// 묶음·펼침이 바뀌면 보이는 행 다시 계산
    fn results_rows(&mut self) {
        let Some(scan) = &self.scan else { return };
        self.res.rows = visible(scan, &self.res.groups, &self.res.open);
        listview::set_count(self.res.list, self.res.rows.len());
        listview::redraw(self.res.list);
    }

    /// 행이 가리키는 검출 (묶음 행은 첫 건)
    fn res_rec(&self, row: usize) -> Option<(Src, usize)> {
        match *self.res.rows.get(row)? {
            Row::Group(g) => self.res.groups.get(g as usize)?.first().copied(),
            Row::Member(g, m) => self.res.groups.get(g as usize)?.get(m as usize).copied(),
        }
    }

    /// 묶음 행 [+]/[−]
    fn results_toggle(&mut self, row: usize) {
        let (Some(scan), Some(Row::Group(g))) = (&self.scan, self.res.rows.get(row).copied()) else { return };
        let grp = &self.res.groups[g as usize];
        if grp.len() < 2 {
            return;
        }
        let k = group_key(scan, grp);
        if !self.res.open.remove(&k) {
            self.res.open.insert(k);
        }
        self.results_rows();
    }

    /// 행들의 원본 값 (중복 제거)
    fn res_values(&self, rows: &[usize]) -> Vec<String> {
        let Some(scan) = &self.scan else { return vec![] };
        let mut v: Vec<String> = rows.iter().filter_map(|i| self.res_rec(*i)).map(|(s, k)| scan.records(s)[k].original.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    pub(super) fn results_notify(&mut self, code: u32, lp: LPARAM) -> Option<LRESULT> {
        match code {
            LVN_GETDISPINFOW => {
                let di = unsafe { &mut *(lp.0 as *mut NMLVDISPINFOW) };
                let row = di.item.iItem as usize;
                // 묶음 크기 (펼친 하위 행은 0)
                let (n, member) = match self.res.rows.get(row) {
                    Some(Row::Group(g)) => (self.res.groups[*g as usize].len(), false),
                    _ => (0, true),
                };
                if di.item.iSubItem == 0 {
                    let st = match (&self.scan, self.res_rec(row)) {
                        (Some(scan), Some((s, k))) => exclude::check([scan.records(s)[k].original.as_str()], &self.excluded),
                        _ => Check::On,
                    };
                    listview::put_state(&mut di.item, st as u32);
                    if (di.item.mask & LVIF_IMAGE).0 != 0 {
                        let open = n > 1 && self.scan.as_ref().zip(self.res.rows.get(row)).is_some_and(|(scan, r)| match r {
                            Row::Group(g) => self.res.open.contains(&group_key(scan, &self.res.groups[*g as usize])),
                            _ => false,
                        });
                        di.item.iImage = if n < 2 { 0 } else if open { 2 } else { 1 };
                    }
                    if (di.item.mask & LVIF_INDENT).0 != 0 {
                        di.item.iIndent = member as i32;
                    }
                }
                if (di.item.mask & LVIF_TEXT).0 != 0 {
                    let t = match (&self.scan, self.res_rec(row)) {
                        (Some(scan), Some((s, k))) => {
                            let r = &scan.records(s)[k];
                            match di.item.iSubItem {
                                0 if n > 1 => format!("{}  ({n}건)", r.original),
                                0 => r.original.clone(),
                                1 if self.excluded.contains(&r.original) => "[제외 — 원본 유지]".into(),
                                1 => r.masked.clone(),
                                2 => r.label.clone(),
                                _ if n > 1 => {
                                    // 묶음: 모두 같은 파일이면 그 파일, 아니면 파일 수
                                    let grp = match self.res.rows.get(row) {
                                        Some(Row::Group(g)) => &self.res.groups[*g as usize],
                                        _ => unreachable!(),
                                    };
                                    let mut files: Vec<&std::path::Path> = grp.iter().map(|(s, _)| scan.path(*s)).collect();
                                    files.dedup();
                                    if files.len() == 1 {
                                        scan.rel(files[0])
                                    } else {
                                        format!("파일 {}개 (펼쳐서 보기)", files.len())
                                    }
                                }
                                _ => scan.rel(scan.path(s)),
                            }
                        }
                        _ => String::new(),
                    };
                    listview::put_text(&mut di.item, &t, &mut self.buf);
                }
                Some(LRESULT(0))
            }
            NM_CUSTOMDRAW => {
                let cd = unsafe { &mut *(lp.0 as *mut NMLVCUSTOMDRAW) };
                Some(LRESULT(match cd.nmcd.dwDrawStage {
                    CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW as isize,
                    CDDS_ITEMPREPAINT => {
                        let (review, off) = match (&self.scan, self.res_rec(cd.nmcd.dwItemSpec)) {
                            (Some(scan), Some((s, k))) => {
                                let r = &scan.records(s)[k];
                                (r.review, self.excluded.contains(&r.original))
                            }
                            _ => (false, false),
                        };
                        if off {
                            cd.clrText = rgb(128, 128, 128);
                        } else if review {
                            cd.clrText = rgb(176, 96, 0);
                        }
                        CDRF_NEWFONT as isize
                    }
                    _ => CDRF_DODEFAULT as isize,
                }))
            }
            NM_CLICK => {
                let ia = unsafe { &*(lp.0 as *const NMITEMACTIVATE) };
                if let Some(row) = listview::state_hit(self.res.list, ia.ptAction) {
                    let vals = self.res_values(&[row]);
                    self.toggle_values(&vals);
                } else if let Some((row, 0, true)) = listview::hit(self.res.list, ia.ptAction) {
                    self.results_toggle(row);
                }
                Some(LRESULT(0))
            }
            NM_DBLCLK => {
                let ia = unsafe { &*(lp.0 as *const NMITEMACTIVATE) };
                if ia.iItem >= 0 {
                    self.results_toggle(ia.iItem as usize);
                }
                Some(LRESULT(0))
            }
            LVN_KEYDOWN => {
                let kd = unsafe { &*(lp.0 as *const NMLVKEYDOWN) };
                if kd.wVKey == 0x20 {
                    // 스페이스: 선택한 행 모두
                    let vals = self.res_values(&listview::selected_rows(self.res.list));
                    self.toggle_values(&vals);
                }
                Some(LRESULT(0))
            }
            NM_RCLICK => {
                let ia = unsafe { &*(lp.0 as *const NMITEMACTIVATE) };
                if let Some(t) = self.res_rec(ia.iItem.max(0) as usize).filter(|_| ia.iItem >= 0) {
                    self.ctx_target = Some(t);
                    let mut sel = listview::selected_rows(self.res.list);
                    if !sel.contains(&(ia.iItem as usize)) {
                        sel = vec![ia.iItem as usize];
                    }
                    let vals = self.res_values(&sel);
                    let mut items = self.exclude_menu(vals);
                    if !items.is_empty() {
                        items.push((0, "-"));
                    }
                    items.extend([(id::C_SRC, "원본 파일 위치 열기"), (id::C_DST, "결과 파일 위치 열기"), (id::C_COPY, "검출된 정보 복사")]);
                    let mut pt = POINT::default();
                    unsafe {
                        let _ = GetCursorPos(&mut pt);
                    }
                    self.acts.push(Act::Popup(pt, items));
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_and_counts() {
        let s = super::super::tree::tests::sample(9); // 검출 파일 3개 × (핸드폰 1 + IP 검토 1)
        assert_eq!(filter(&s, false, None, "").len(), 6);
        assert_eq!(filter(&s, true, None, "").len(), 3);
        assert_eq!(filter(&s, false, Some("핸드폰번호"), "").len(), 3);
        assert_eq!(filter(&s, false, None, "f003").len(), 2);
        assert_eq!(counts(&s), vec![("IP주소".to_string(), 3), ("핸드폰번호".to_string(), 3)]);
    }

    #[test]
    fn duplicates_grouped_and_expand_on_demand() {
        let s = super::super::tree::tests::sample(9); // "10.0.0.1"(IP, 검토필요) 가 파일 3개에 1건씩, 핸드폰은 모두 다른 값
        let g = group(&s, &filter(&s, false, None, ""));
        assert_eq!(g.len(), 4, "IP 3건이 한 묶음 + 핸드폰 3개");
        let ip = g.iter().find(|x| x.len() == 3).expect("IP 묶음");
        assert!(ip.iter().all(|(src, k)| s.records(*src)[*k].original == "10.0.0.1"));
        let mut open = std::collections::HashSet::new();
        assert_eq!(visible(&s, &g, &open).len(), 4, "접힌 상태: 묶음 1행");
        open.insert(group_key(&s, ip));
        assert_eq!(visible(&s, &g, &open).len(), 7, "펼치면 3건이 아래에");
    }
}

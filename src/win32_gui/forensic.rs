//! 파일 분석 탭 — Process Explorer 식 트리 목록(가상 리스트뷰 + 들여쓰기 + [+]/[−] 이미지) + NetworkMiner 식 상세 창.
//! 행 데이터는 `tree` 모델이 만들고, 리스트뷰는 보이는 칸만 묻는다. 선택이 바뀌면 상세 창을 즉시 갱신한다.

use super::listview::{self, col, col_r, Col};
use super::tree::{self, human, Kind, Node};
use super::exclude::{self, Check};
use super::{ctl, dpi::px, gdi, id, layout, rgb, Act, App, FontKind};
use crate::gui::scan::Src;
use crate::ipc::Status;
use std::collections::HashSet;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const TREE_COLS: [Col; 7] = [
    col("이름", 280),
    col("형식 (매직바이트)", 120),
    col_r("크기", 80),
    col_r("검출", 50),
    col_r("검토", 50),
    col("상태", 64),
    col("비고", 0),
];
const REC_COLS: [Col; 4] = [col("검출된정보", 180), col("치환정보", 180), col("정보타입", 140), col("파일경로", 0)];
const PROP_COLS: [Col; 2] = [col("항목", 160), col("값", 0)];

#[derive(Clone, PartialEq)]
enum Sel {
    File(usize),
    Dir(String),
}

pub struct Page {
    expand: HWND,
    collapse: HWND,
    found: HWND,
    search: HWND,
    legend: HWND,
    pub(super) tree: HWND,
    title: HWND,
    tab_recs: HWND,
    tab_props: HWND,
    pub(super) recs: HWND,
    props: HWND,
    btn_src: HWND,
    btn_dst: HWND,
    nodes: Vec<Node>,
    vis: Vec<u32>,
    collapsed: HashSet<String>,
    expanded: HashSet<usize>,
    expand_ver: u64,
    key: Option<(u64, bool, String, u64)>,
    selected: Option<Sel>,
    show_props: bool,
    det: Vec<(Src, usize)>,
    /// 상세 창 높이 (96 DPI 기준)
    split: i32,
    pub drag: bool,
    boxes: Option<gdi::ImageList>,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            expand: HWND::default(),
            collapse: HWND::default(),
            found: HWND::default(),
            search: HWND::default(),
            legend: HWND::default(),
            tree: HWND::default(),
            title: HWND::default(),
            tab_recs: HWND::default(),
            tab_props: HWND::default(),
            recs: HWND::default(),
            props: HWND::default(),
            btn_src: HWND::default(),
            btn_dst: HWND::default(),
            nodes: vec![],
            vis: vec![],
            collapsed: HashSet::new(),
            expanded: HashSet::new(),
            expand_ver: 0,
            key: None,
            selected: None,
            show_props: false,
            det: vec![],
            split: 220,
            drag: false,
            boxes: None,
        }
    }
}

impl Page {
    /// 새 검사 시작: 트리 상태 초기화 (컨트롤·창 크기 설정은 유지)
    pub fn reset(&mut self) {
        self.nodes.clear();
        self.vis.clear();
        self.collapsed.clear();
        self.expanded.clear();
        self.key = None;
        self.selected = None;
        self.det.clear();
    }
}

/// 상태별 행 배경 (Process Explorer 식)
fn tint(n: &Node, scan: &crate::gui::scan::Scan) -> Option<COLORREF> {
    match &n.kind {
        Kind::Dir(_) => (n.agg.recs > 0).then(|| rgb(208, 228, 255)),
        Kind::File(i) => {
            let e = &scan.entries[*i];
            if n.agg.review > 0 {
                return Some(rgb(255, 236, 160));
            }
            match e.status {
                Status::Masked => Some(rgb(255, 208, 208)),
                Status::Found => Some(rgb(226, 210, 255)),
                Status::Skipped => Some(rgb(222, 222, 222)),
                Status::Error => Some(rgb(170, 170, 170)),
                Status::Clean => None,
            }
        }
        _ => None,
    }
}

impl App {
    pub(super) fn forensic_create(&mut self) {
        let (h, dpi) = (self.hwnd, self.dpi());
        let reg = &mut self.reg;
        let p = &mut self.fx;
        p.expand = ctl::button(h, "모두 펼치기", id::F_EXPAND, reg);
        p.collapse = ctl::button(h, "모두 접기", id::F_COLLAPSE, reg);
        p.found = ctl::checkbox(h, "정보가 있는 파일만", id::F_FOUND, reg);
        p.search = ctl::edit(h, id::F_SEARCH, 0, reg, FontKind::Ui);
        ctl::cue(p.search, "파일 이름·경로 검색");
        // 폭이 모자라도 두 줄로 접혀 잘리지 않게 한 줄 + 끝 말줄임
        p.legend = ctl::child(h, ctl::STATIC, "", 0x80 /* SS_NOPREFIX */ | 0xC /* SS_LEFTNOWORDWRAP */ | 0x4000 /* SS_ENDELLIPSIS */, 0, 0, reg, FontKind::Ui);
        p.tree = listview::create(h, id::F_TREE, true, 0, &TREE_COLS, dpi, reg, FontKind::Ui);
        p.title = ctl::label(h, "", reg, FontKind::Bold);
        // 상세 창 탭: 누름 모양 라디오 버튼 2개
        p.tab_recs = ctl::child(h, ctl::BUTTON, "검출 정보", WS_TABSTOP.0 | WS_GROUP.0 | 9 | 0x1000, 0, id::F_TAB_RECS, reg, FontKind::Ui);
        p.tab_props = ctl::child(h, ctl::BUTTON, "파일 정보", WS_TABSTOP.0 | 9 | 0x1000, 0, id::F_TAB_PROPS, reg, FontKind::Ui);
        ctl::set_check(p.tab_recs, true);
        p.recs = listview::create(h, id::F_RECS, true, 0, &REC_COLS, dpi, reg, FontKind::Ui);
        listview::multi_select(p.recs);
        p.props = listview::create(h, id::F_PROPS, false, 0, &PROP_COLS, dpi, reg, FontKind::Ui);
        p.btn_src = ctl::button(h, "원본 위치 열기", id::F_SRC, reg);
        p.btn_dst = ctl::button(h, "결과 위치 열기", id::F_DST, reg);
    }

    /// [+]/[−] 이미지 (DPI 별로 다시 만들고, 이전 목록은 Drop 으로 해제)
    pub(super) fn forensic_images(&mut self) {
        let list = gdi::tree_boxes(px(16, self.dpi()));
        ctl::send(self.fx.tree, LVM_SETIMAGELIST, LVSIL_SMALL as usize, list.handle().0 as isize);
        ctl::send(self.res.list, LVM_SETIMAGELIST, LVSIL_SMALL as usize, list.handle().0 as isize); // 검출 결과 묶음 [+]
        self.fx.boxes = Some(list);
    }

    fn fx_layout(&self) -> layout::Forensic {
        layout::forensic(self.page(), px(self.fx.split, self.dpi()), self.dpi())
    }

    pub(super) fn forensic_layout(&self, pg: layout::R) {
        let dpi = self.dpi();
        let l = layout::forensic(pg, px(self.fx.split, dpi), dpi);
        let p = &self.fx;
        for (h, r) in [
            (p.expand, l.expand),
            (p.collapse, l.collapse),
            (p.found, l.only_found),
            (p.search, l.search),
            (p.legend, l.legend),
            (p.tree, l.tree),
            (p.title, l.title),
            (p.tab_recs, l.tab_recs),
            (p.tab_props, l.tab_props),
            (p.recs, layout::R::new(l.detail.x, l.detail.y, l.detail.w, l.btn_src.bottom() - l.detail.y)),
            (p.props, l.detail),
            (p.btn_src, l.btn_src),
            (p.btn_dst, l.btn_dst),
        ] {
            ctl::place(h, r);
        }
        listview::fit_columns(p.tree, &TREE_COLS, l.tree.w, dpi);
        listview::fit_columns(p.recs, &REC_COLS, l.detail.w, dpi);
        listview::fit_columns(p.props, &PROP_COLS, l.detail.w, dpi);
    }

    pub(super) fn forensic_columns(&self) {
        self.forensic_layout(self.page());
    }

    pub(super) fn forensic_show(&self, on: bool) {
        let p = &self.fx;
        for h in [p.expand, p.collapse, p.found, p.search, p.legend, p.tree, p.title, p.tab_recs, p.tab_props] {
            ctl::show(h, on);
        }
        let props = p.show_props && matches!(p.selected, Some(Sel::File(_)) | Some(Sel::Dir(_)));
        ctl::show(p.recs, on && !props);
        ctl::show(p.props, on && props);
        let file = matches!(p.selected, Some(Sel::File(_)));
        ctl::show(p.btn_src, on && props && file);
        ctl::show(p.btn_dst, on && props && file);
    }

    pub(super) fn fx_splitter_hit(&self, pt: POINT) -> bool {
        let s = self.fx_layout().splitter;
        let pad = px(2, self.dpi());
        layout::R::new(s.x, s.y - pad, s.w, s.h + 2 * pad).contains(pt.x, pt.y)
    }

    pub(super) fn fx_set_split(&mut self, y: i32) {
        let pg = self.page();
        self.fx.split = ((pg.bottom() - y).max(0) as i64 * 96 / self.dpi() as i64) as i32;
        self.forensic_layout(pg);
    }

    /// 트리 다시 만들기 (검사 결과·필터·파일 펼침이 바뀌었을 때만)
    pub(super) fn forensic_refresh(&mut self) {
        let Some(scan) = &self.scan else {
            ctl::set_text(self.fx.legend, "검사를 시작하면 검사한 모든 파일이 폴더 구조 그대로 표시됩니다.");
            listview::set_count(self.fx.tree, 0);
            self.forensic_detail();
            return;
        };
        let key = (scan.version, ctl::checked(self.fx.found), ctl::text(self.fx.search).trim().to_lowercase(), self.fx.expand_ver);
        if self.fx.key.as_ref() == Some(&key) {
            return;
        }
        let total: u64 = scan.entries.iter().map(|e| e.size).sum();
        ctl::set_text(
            self.fx.legend,
            &format!("파일 {}개 · {}    분홍 치환 · 노랑 검토필요 · 보라 검출 · 회색 제외 · 하늘 정보가 든 폴더", scan.entries.len(), human(total)),
        );
        self.fx.nodes = tree::build(scan, key.1, &key.2, &self.fx.expanded);
        self.fx.key = Some(key);
        self.forensic_revisible();
    }

    /// 접기 상태만 바뀌었을 때 (모델은 그대로)
    fn forensic_revisible(&mut self) {
        let p = &mut self.fx;
        p.vis = tree::visible(&p.nodes, &p.collapsed);
        listview::set_count(p.tree, p.vis.len());
        // 선택 유지: 선택했던 파일·폴더 행을 다시 선택
        ctl::send(p.tree, LVM_SETITEMSTATE, usize::MAX, &LVITEMW { stateMask: LVIS_SELECTED, ..Default::default() } as *const _ as isize);
        let row = p.selected.as_ref().and_then(|sel| {
            p.vis.iter().position(|&i| match (&p.nodes[i as usize].kind, sel) {
                (Kind::File(a), Sel::File(b)) => a == b,
                (Kind::Dir(a), Sel::Dir(b)) => a == b,
                _ => false,
            })
        });
        if let Some(r) = row {
            listview::select(p.tree, r);
        }
        self.forensic_detail();
    }

    fn node_at(&self, row: usize) -> Option<&Node> {
        self.fx.vis.get(row).map(|&i| &self.fx.nodes[i as usize])
    }

    /// 트리 행의 제외 대상 값: 값 행은 그 값, 타입 묶음은 묶음 안의 값 전부 (그 밖의 행은 없음)
    fn tree_values(&self, row: usize) -> Vec<String> {
        let (Some(scan), Some(&at)) = (&self.scan, self.fx.vis.get(row)) else { return vec![] };
        let at = at as usize;
        let n = &self.fx.nodes[at];
        let item = |n: &Node| match n.kind {
            Kind::Item(_, s, k) => Some(scan.records(s)[k].original.clone()),
            _ => None,
        };
        let mut v: Vec<String> = match n.kind {
            Kind::Item(..) => item(n).into_iter().collect(),
            Kind::Type(..) => self.fx.nodes[at + 1..].iter().take_while(|c| c.depth > n.depth).filter_map(item).collect(),
            _ => vec![],
        };
        v.sort();
        v.dedup();
        v
    }

    /// 상세 창 검출 목록 행들의 원본 값 (중복 제거)
    fn recs_values(&self, rows: &[usize]) -> Vec<String> {
        let Some(scan) = &self.scan else { return vec![] };
        let mut v: Vec<String> = rows.iter().filter_map(|i| self.fx.det.get(*i)).map(|&(s, k)| scan.records(s)[k].original.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    fn popup(&mut self, items: Vec<(u16, &'static str)>) {
        if items.is_empty() {
            return;
        }
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        self.acts.push(Act::Popup(pt, items));
    }

    fn forensic_toggle(&mut self, row: usize) {
        let Some(n) = self.node_at(row) else { return };
        match n.kind.clone() {
            Kind::Dir(k) | Kind::Type(k, _) => {
                if !self.fx.collapsed.remove(&k) {
                    self.fx.collapsed.insert(k);
                }
                self.forensic_revisible();
            }
            Kind::File(i) if n.expandable() => {
                if !self.fx.expanded.remove(&i) {
                    self.fx.expanded.insert(i);
                }
                self.fx.expand_ver += 1;
                self.fx.selected = Some(Sel::File(i));
                self.forensic_refresh();
            }
            _ => {}
        }
    }

    fn forensic_select(&mut self, row: usize) {
        let Some(n) = self.node_at(row) else { return };
        // 타입 묶음·값 행은 그 파일을 선택한 것으로
        let sel = match &n.kind {
            Kind::Dir(p) => Sel::Dir(p.clone()),
            Kind::File(i) | Kind::Type(_, i) | Kind::Item(i, ..) => Sel::File(*i),
        };
        if self.fx.selected.as_ref() != Some(&sel) {
            self.fx.selected = Some(sel);
            self.forensic_detail();
        }
    }

    /// 아래 상세 창 (선택이 바뀌면 즉시)
    fn forensic_detail(&mut self) {
        let Some(scan) = &self.scan else {
            ctl::set_text(self.fx.title, "");
            self.fx.det.clear();
            listview::set_count(self.fx.recs, 0);
            listview::clear(self.fx.props);
            return;
        };
        let (title, det): (String, Vec<(Src, usize)>) = match &self.fx.selected {
            None => ("파일이나 폴더를 고르면 찾은 정보와 파일 정보가 표시됩니다. ([+] 또는 두 번 클릭: 펼치기)".into(), vec![]),
            Some(Sel::File(i)) => {
                let e = &scan.entries[*i];
                let mut v: Vec<(Src, usize)> = (0..e.records.len()).map(|k| (Src::File(*i), k)).collect();
                if let Some(n) = scan.names.iter().position(|n| n.path == e.path) {
                    v.extend((0..scan.names[n].records.len()).map(|k| (Src::Name(n), k)));
                }
                (scan.rel(&e.path), v)
            }
            Some(Sel::Dir(d)) => {
                let prefix = format!("{d}{}", std::path::MAIN_SEPARATOR);
                let under = |p: &std::path::Path| {
                    let r = scan.rel(p);
                    r == *d || r.starts_with(&prefix)
                };
                let mut v = Vec::new();
                for (i, e) in scan.entries.iter().enumerate().filter(|(_, e)| under(&e.path)) {
                    v.extend((0..e.records.len()).map(|k| (Src::File(i), k)));
                }
                for (i, n) in scan.names.iter().enumerate().filter(|(_, n)| under(&n.path)) {
                    v.extend((0..n.records.len()).map(|k| (Src::Name(i), k)));
                }
                (prefix, v)
            }
        };
        ctl::set_text(self.fx.title, &title);
        ctl::set_text(self.fx.tab_recs, &format!("검출 정보 ({})", det.len()));
        self.fx.det = det;
        listview::set_count(self.fx.recs, self.fx.det.len());

        // 파일 정보
        listview::clear(self.fx.props);
        let rows: Vec<(String, String)> = match &self.fx.selected {
            Some(Sel::File(i)) => {
                let e = &scan.entries[*i];
                let name = e.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let ext = crate::exts::ext_of(&name);
                let cat = ext.as_deref().and_then(crate::exts::lookup).map(|x| format!("{} — {}", x.category, x.desc)).unwrap_or_else(|| "목록에 없음".into());
                let review = e.records.iter().filter(|r| r.review).count();
                let mut v = vec![
                    ("원본 경로".into(), e.path.display().to_string()),
                    ("결과 경로".into(), e.out.display().to_string()),
                    ("실제 형식 (매직바이트)".into(), e.kind.clone()),
                    ("확장자".into(), ext.map(|x| format!(".{x}")).unwrap_or_else(|| "없음".into())),
                    ("확장자 분류".into(), cat),
                    ("크기".into(), format!("{} ({} bytes)", human(e.size), e.size)),
                    ("상태".into(), e.status.label().into()),
                    ("검출".into(), format!("{}건 (검토필요 {review}건)", e.records.len())),
                ];
                v.extend(e.notes.iter().map(|n| ("알림".to_string(), n.clone())));
                v
            }
            Some(Sel::Dir(d)) => {
                let agg = self.fx.nodes.iter().find(|n| matches!(&n.kind, Kind::Dir(x) if x == d)).map(|n| n.agg).unwrap_or_default();
                vec![
                    ("원본 경로".into(), scan.root.join(d).display().to_string()),
                    ("파일 수".into(), agg.files.to_string()),
                    ("전체 크기".into(), human(agg.size)),
                    ("치환한 파일".into(), agg.masked.to_string()),
                    ("검출".into(), format!("{}건 (검토필요 {}건)", agg.recs, agg.review)),
                ]
            }
            None => vec![],
        };
        for (k, v) in &rows {
            listview::push(self.fx.props, &[k, v]);
        }
        self.forensic_show(self.tab == super::T_FORENSIC);
    }

    pub(super) fn forensic_command(&mut self, cid: u16, code: u32) {
        match (cid, code) {
            (id::F_EXPAND, BN_CLICKED) => {
                self.fx.collapsed.clear();
                if let Some(scan) = &self.scan {
                    self.fx.expanded = scan.entries.iter().enumerate().filter(|(_, e)| !e.records.is_empty()).map(|(i, _)| i).collect();
                }
                self.fx.expand_ver += 1;
                self.forensic_refresh();
            }
            (id::F_COLLAPSE, BN_CLICKED) => {
                self.fx.collapsed = self.fx.nodes.iter().filter_map(|n| if let Kind::Dir(p) = &n.kind { Some(p.clone()) } else { None }).collect();
                self.fx.expanded.clear();
                self.fx.expand_ver += 1;
                self.forensic_refresh();
            }
            (id::F_FOUND, BN_CLICKED) | (id::F_SEARCH, EN_CHANGE) => self.forensic_refresh(),
            (id::F_TAB_RECS | id::F_TAB_PROPS, BN_CLICKED) => {
                self.fx.show_props = cid == id::F_TAB_PROPS;
                self.forensic_show(true);
            }
            (id::F_SRC | id::F_DST, BN_CLICKED) => {
                if let (Some(scan), Some(Sel::File(i))) = (&self.scan, &self.fx.selected) {
                    let e = &scan.entries[*i];
                    self.acts.push(Act::Reveal(if cid == id::F_SRC { e.path.clone() } else { e.out.clone() }));
                }
            }
            _ => {}
        }
    }

    pub(super) fn forensic_notify(&mut self, cid: u16, code: u32, lp: LPARAM) -> Option<LRESULT> {
        match (cid, code) {
            (id::F_TREE, LVN_GETDISPINFOW) => {
                let di = unsafe { &mut *(lp.0 as *mut NMLVDISPINFOW) };
                let row = di.item.iItem as usize;
                let (Some(scan), Some(n)) = (&self.scan, self.fx.vis.get(row).map(|&i| &self.fx.nodes[i as usize])) else { return Some(LRESULT(0)) };
                if di.item.iSubItem == 0 {
                    let st = match n.kind {
                        Kind::Item(..) | Kind::Type(..) => {
                            let vals = self.tree_values(row);
                            exclude::check(vals.iter().map(String::as_str), &self.excluded) as u32
                        }
                        _ => 0, // 폴더·파일 행은 상자 없음
                    };
                    listview::put_state(&mut di.item, st);
                    if (di.item.mask & LVIF_INDENT).0 != 0 {
                        di.item.iIndent = n.depth as i32;
                    }
                    if (di.item.mask & LVIF_IMAGE).0 != 0 {
                        let open = match &n.kind {
                            Kind::Dir(k) | Kind::Type(k, _) => !self.fx.collapsed.contains(k),
                            Kind::File(i) => self.fx.expanded.contains(i),
                            Kind::Item(..) => false,
                        };
                        di.item.iImage = if !n.expandable() { 0 } else if open { 2 } else { 1 };
                    }
                }
                if (di.item.mask & LVIF_TEXT).0 != 0 {
                    let own = matches!(n.kind, Kind::Dir(_) | Kind::File(_));
                    let t = match di.item.iSubItem {
                        0 if matches!(n.kind, Kind::Item(..)) => match n.kind {
                            Kind::Item(_, s, k) => {
                                let r = &scan.records(s)[k];
                                let m = if self.excluded.contains(&r.original) { "[제외 — 원본 유지]" } else { r.masked.as_str() };
                                format!("{} → {m}", r.original)
                            }
                            _ => unreachable!(),
                        },
                        0 => {
                            if n.name_hit.is_some() {
                                format!("{}  [이름에 정보]", n.name)
                            } else {
                                n.name.clone()
                            }
                        }
                        1 => match &n.kind {
                            Kind::Dir(_) => "폴더".into(),
                            Kind::File(i) => scan.entries[*i].kind.clone(),
                            _ => String::new(),
                        },
                        2 if own => human(n.agg.size),
                        3 if !matches!(n.kind, Kind::Item(..)) && n.agg.recs > 0 => n.agg.recs.to_string(),
                        4 if !matches!(n.kind, Kind::Item(..)) && n.agg.review > 0 => n.agg.review.to_string(),
                        5 => match &n.kind {
                            Kind::File(i) => scan.entries[*i].status.label().into(),
                            Kind::Item(..) if n.agg.review > 0 => "검토필요".into(),
                            _ => String::new(),
                        },
                        6 => match &n.kind {
                            Kind::Dir(_) => format!("파일 {}개", n.agg.files),
                            Kind::File(i) => scan.entries[*i].notes.join(" / "),
                            Kind::Item(_, s, k) => scan.records(*s)[*k].label.clone(),
                            Kind::Type(..) => String::new(),
                        },
                        _ => String::new(),
                    };
                    listview::put_text(&mut di.item, &t, &mut self.buf);
                }
                Some(LRESULT(0))
            }
            (id::F_TREE, NM_CUSTOMDRAW) => {
                let cd = unsafe { &mut *(lp.0 as *mut NMLVCUSTOMDRAW) };
                Some(LRESULT(match cd.nmcd.dwDrawStage {
                    CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW as isize,
                    CDDS_ITEMPREPAINT => {
                        if let (Some(scan), Some(n)) = (&self.scan, self.node_at(cd.nmcd.dwItemSpec)) {
                            if let Some(bg) = tint(n, scan) {
                                cd.clrTextBk = bg;
                            }
                            if let Kind::Item(_, s, k) = n.kind {
                                if self.excluded.contains(&scan.records(s)[k].original) {
                                    cd.clrText = rgb(128, 128, 128);
                                } else if n.agg.review > 0 {
                                    cd.clrText = rgb(176, 96, 0);
                                }
                            }
                        }
                        CDRF_NEWFONT as isize
                    }
                    _ => CDRF_DODEFAULT as isize,
                }))
            }
            (id::F_TREE, NM_CLICK) => {
                let ia = unsafe { &*(lp.0 as *const NMITEMACTIVATE) };
                if let Some(row) = listview::state_hit(self.fx.tree, ia.ptAction) {
                    let vals = self.tree_values(row);
                    self.toggle_values(&vals);
                } else if let Some((row, 0, true)) = listview::hit(self.fx.tree, ia.ptAction) {
                    self.forensic_toggle(row);
                }
                Some(LRESULT(0))
            }
            (id::F_TREE, LVN_KEYDOWN) => {
                let kd = unsafe { &*(lp.0 as *const NMLVKEYDOWN) };
                if kd.wVKey == 0x20 {
                    if let Some(&row) = listview::selected_rows(self.fx.tree).first() {
                        let vals = self.tree_values(row);
                        self.toggle_values(&vals);
                    }
                }
                Some(LRESULT(0))
            }
            (id::F_TREE, NM_RCLICK) => {
                let ia = unsafe { &*(lp.0 as *const NMITEMACTIVATE) };
                if ia.iItem >= 0 {
                    let row = ia.iItem as usize;
                    let vals = self.tree_values(row);
                    let mut items = self.exclude_menu(vals);
                    if let Some(Kind::Item(_, s, k)) = self.node_at(row).map(|n| n.kind.clone()) {
                        self.ctx_target = Some((s, k));
                        if !items.is_empty() {
                            items.push((0, "-"));
                        }
                        items.extend([(id::C_SRC, "원본 파일 위치 열기"), (id::C_DST, "결과 파일 위치 열기"), (id::C_COPY, "검출된 정보 복사")]);
                    }
                    self.popup(items);
                }
                Some(LRESULT(0))
            }
            (id::F_TREE, NM_DBLCLK) => {
                let ia = unsafe { &*(lp.0 as *const NMITEMACTIVATE) };
                if ia.iItem >= 0 {
                    let row = ia.iItem as usize;
                    match self.node_at(row).map(|n| n.kind.clone()) {
                        Some(Kind::File(i)) if self.node_at(row).is_some_and(|n| !n.expandable()) => {
                            if let Some(scan) = &self.scan {
                                self.acts.push(Act::Reveal(scan.entries[i].out.clone()));
                            }
                        }
                        Some(Kind::Item(..)) | None => {}
                        Some(_) => self.forensic_toggle(row),
                    }
                }
                Some(LRESULT(0))
            }
            (id::F_TREE, LVN_ITEMCHANGED) => {
                let nl = unsafe { &*(lp.0 as *const NMLISTVIEW) };
                let now = (nl.uNewState & LVIS_SELECTED.0) != 0;
                let before = (nl.uOldState & LVIS_SELECTED.0) != 0;
                if (nl.uChanged & LVIF_STATE).0 != 0 && now && !before && nl.iItem >= 0 {
                    self.forensic_select(nl.iItem as usize);
                }
                Some(LRESULT(0))
            }
            (id::F_RECS, LVN_GETDISPINFOW) => {
                let di = unsafe { &mut *(lp.0 as *mut NMLVDISPINFOW) };
                if di.item.iSubItem == 0 {
                    let st = match (&self.scan, self.fx.det.get(di.item.iItem as usize)) {
                        (Some(scan), Some(&(s, k))) => exclude::check([scan.records(s)[k].original.as_str()], &self.excluded),
                        _ => Check::On,
                    };
                    listview::put_state(&mut di.item, st as u32);
                }
                if (di.item.mask & LVIF_TEXT).0 != 0 {
                    let t = match (&self.scan, self.fx.det.get(di.item.iItem as usize)) {
                        (Some(scan), Some(&(s, k))) => {
                            let r = &scan.records(s)[k];
                            match di.item.iSubItem {
                                0 => r.original.clone(),
                                1 if self.excluded.contains(&r.original) => "[제외 — 원본 유지]".into(),
                                1 => r.masked.clone(),
                                2 => r.label.clone(),
                                _ => scan.rel(scan.path(s)),
                            }
                        }
                        _ => String::new(),
                    };
                    listview::put_text(&mut di.item, &t, &mut self.buf);
                }
                Some(LRESULT(0))
            }
            (id::F_RECS, NM_CUSTOMDRAW) => {
                let cd = unsafe { &mut *(lp.0 as *mut NMLVCUSTOMDRAW) };
                Some(LRESULT(match cd.nmcd.dwDrawStage {
                    CDDS_PREPAINT => CDRF_NOTIFYITEMDRAW as isize,
                    CDDS_ITEMPREPAINT => {
                        if let (Some(scan), Some(&(s, k))) = (&self.scan, self.fx.det.get(cd.nmcd.dwItemSpec)) {
                            let r = &scan.records(s)[k];
                            if self.excluded.contains(&r.original) {
                                cd.clrText = rgb(128, 128, 128);
                            } else if r.review {
                                cd.clrText = rgb(176, 96, 0);
                            }
                        }
                        CDRF_NEWFONT as isize
                    }
                    _ => CDRF_DODEFAULT as isize,
                }))
            }
            (id::F_RECS, NM_CLICK) => {
                let ia = unsafe { &*(lp.0 as *const NMITEMACTIVATE) };
                if let Some(row) = listview::state_hit(self.fx.recs, ia.ptAction) {
                    let vals = self.recs_values(&[row]);
                    self.toggle_values(&vals);
                }
                Some(LRESULT(0))
            }
            (id::F_RECS, LVN_KEYDOWN) => {
                let kd = unsafe { &*(lp.0 as *const NMLVKEYDOWN) };
                if kd.wVKey == 0x20 {
                    let vals = self.recs_values(&listview::selected_rows(self.fx.recs));
                    self.toggle_values(&vals);
                }
                Some(LRESULT(0))
            }
            (id::F_RECS, NM_RCLICK) => {
                let ia = unsafe { &*(lp.0 as *const NMITEMACTIVATE) };
                if let Some(&t) = self.fx.det.get(ia.iItem.max(0) as usize).filter(|_| ia.iItem >= 0) {
                    self.ctx_target = Some(t);
                    let mut sel = listview::selected_rows(self.fx.recs);
                    if !sel.contains(&(ia.iItem as usize)) {
                        sel = vec![ia.iItem as usize];
                    }
                    let vals = self.recs_values(&sel);
                    let mut items = self.exclude_menu(vals);
                    if !items.is_empty() {
                        items.push((0, "-"));
                    }
                    items.extend([(id::C_SRC, "원본 파일 위치 열기"), (id::C_DST, "결과 파일 위치 열기"), (id::C_COPY, "검출된 정보 복사")]);
                    self.popup(items);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }
}

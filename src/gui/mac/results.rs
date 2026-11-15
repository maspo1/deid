//! 검출 결과 탭 — 보고서(CSV)와 같은 내용을 사람이 검토하는 표. 검토필요 행은 주황 글자.
//! 같은 값·같은 정보타입은 한 행으로 묶고(두 번 눌러 펼침), 행마다 체크 상자: 해제하면 그 값(같은 값 모두)은 치환하지 않는다.
//! 우클릭 → "이 형식은 제외" 는 같은 정보타입에서 숫자만 다른 값(1 0 0 : 1 ≡ 2 5 5 : 3)을 모두 제외한다.
//! 제외는 엔진에 넘겨 다시 검사해야 결과 파일에 반영된다 ("제외 반영해 다시 검사").

use super::kit::{self, cell, checkbox, col_id, is_on, label, note, set_on, Look, H, PLAIN};
use super::{st, st_mut, ui, Controller};
use crate::config::format_key;
use crate::gui::scan::{Scan, Src};
use crate::gui::sys;
use crate::ipc::Rec;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{sel, MainThreadOnly};
use objc2_app_kit::{
    NSButton, NSMenu, NSMenuItem, NSPopUpButton, NSScrollView, NSSearchField, NSStackView, NSTableColumn, NSTableView, NSTextField,
    NSUserInterfaceItemIdentification, NSView,
};
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct ResultsUi {
    pub view: Retained<NSView>,
    summary: Retained<NSTextField>,
    review_only: Retained<NSButton>,
    kind: Retained<NSPopUpButton>,
    search: Retained<NSSearchField>,
    warn: Retained<NSTextField>,
    excl_bar: Retained<NSStackView>,
    excl_label: Retained<NSTextField>,
    rescan: Retained<NSButton>,
    scroll: Retained<NSScrollView>,
    pub table: Retained<NSTableView>,
    empty: Retained<NSTextField>,
}

/// 보이는 행: 묶음(1건이면 그냥 한 행) 또는 펼친 묶음 안의 1건
#[derive(Clone, Copy)]
enum Row {
    Group(usize),
    Member(usize, usize),
}

#[derive(Default)]
pub struct ResState {
    /// 필터에 맞는 검출을 (원본 값, 정보타입) 으로 묶은 것 — 처음 나온 순서
    groups: Vec<Vec<(Src, usize)>>,
    /// 펼친 묶음 (원본 값, 정보타입) — 필터·재검사 후에도 유지
    open: HashSet<(String, String)>,
    rows: Vec<Row>,
}

/// 사용자가 제외한 값·형식 (프로그램 메모리에만 둠) — 검사할 때 엔진에 넘긴다
#[derive(Default)]
pub struct Excl {
    pub values: HashSet<String>,
    pub formats: HashSet<String>,
    /// 지금 보이는 결과를 만든 검사에 넘긴 제외 (다르면 다시 검사해야 반영됨)
    pub applied: (HashSet<String>, HashSet<String>),
}

impl Excl {
    fn format_of(r: &Rec) -> String {
        format_key(r.base(), &r.original)
    }
    /// 이 검출이 제외 대상인가 → 제외 이유
    fn reason(&self, r: &Rec) -> Option<&'static str> {
        if self.values.contains(&r.original) {
            Some("[제외 — 원본 유지]")
        } else if self.formats.contains(&Self::format_of(r)) {
            Some("[형식 제외 — 원본 유지]")
        } else {
            None
        }
    }
    fn pending(&self) -> bool {
        self.values != self.applied.0 || self.formats != self.applied.1
    }
}

fn menu_item(title: &str, action: objc2::runtime::Sel, t: &AnyObject) -> Retained<NSMenuItem> {
    let m = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(kit::mtm()), &kit::ns(title), Some(action), &kit::ns("")) };
    unsafe { m.setTarget(Some(t)) };
    m
}

impl ResultsUi {
    pub fn build(ctrl: &Controller) -> ResultsUi {
        let t: &AnyObject = ctrl.as_ref();
        let f = sel!(resultsFilter:);
        let summary = kit::bold_label("");
        // 타입이 많아 길어져도 탭 폭을 밀어내지 않고 말줄임
        summary.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
        kit::stretch(&summary);
        let review_only = checkbox("검토필요만", t, f);
        let kind = NSPopUpButton::new(kit::mtm());
        unsafe {
            kind.setTarget(Some(t));
            kind.setAction(Some(f));
        }
        kind.addItemWithTitle(&kit::ns("모든 타입"));
        let search = kit::search("검출 정보·경로 검색", t, f);
        let filters = kit::hstack(&[&review_only, &kind, &search, &kit::spacer()]);
        let warn = note("※ 애매한 항목도 일단 치환했습니다. 주황색 행을 확인하고, 정보가 아니면 체크를 해제하거나 우클릭 → 제외한 뒤 다시 검사하세요.");
        warn.setTextColor(Some(&kit::warn_color()));
        let excl_label = kit::small_label("");
        let rescan = kit::button("제외 반영해 다시 검사", t, sel!(resRescan:));
        let clear = kit::button("제외 목록 비우기", t, sel!(resClearExcl:));
        let excl_bar = kit::hstack(&[&excl_label, &kit::spacer(), &clear, &rescan]);
        let (scroll, table) = kit::table(&[
            ("chk", "", 26.0),
            ("orig", "검출된정보", 190.0),
            ("masked", "치환정보", 170.0),
            ("label", "정보타입", 130.0),
            ("path", "파일경로", 280.0),
        ]);
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(ctrl)));
            table.setDelegate(Some(ProtocolObject::from_ref(ctrl)));
            table.setTarget(Some(t));
            table.setDoubleAction(Some(sel!(resDoubleClick:)));
        }
        table.setAllowsMultipleSelection(true);
        let menu = NSMenu::new(kit::mtm());
        menu.addItem(&menu_item("제외 (같은 값 모두)", sel!(resExclude:), t));
        menu.addItem(&menu_item("이 형식은 제외 (숫자만 다른 값 모두)", sel!(resExcludeFormat:), t));
        menu.addItem(&menu_item("제외 취소", sel!(resInclude:), t));
        menu.addItem(&NSMenuItem::separatorItem(kit::mtm()));
        menu.addItem(&menu_item("원본 파일 위치 열기", sel!(resRevealSrc:), t));
        menu.addItem(&menu_item("결과 파일 위치 열기", sel!(resRevealOut:), t));
        menu.addItem(&menu_item("검출된 정보 복사", sel!(resCopy:), t));
        unsafe { table.setMenu(Some(&menu)) };
        let empty = label("검사를 마치면 검출된 정보가 여기에 표시됩니다. 보고서(CSV)와 같은 내용입니다.");
        empty.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
        let view = kit::container();
        kit::column(&view, &[(&summary, H::Auto), (&filters, H::Auto), (&warn, H::Auto), (&excl_bar, H::Auto), (&scroll, H::Fill)], 8.0, 6.0);
        empty.setTranslatesAutoresizingMaskIntoConstraints(false);
        view.addSubview(&empty);
        empty.centerXAnchor().constraintEqualToAnchor(&view.centerXAnchor()).setActive(true);
        empty.centerYAnchor().constraintEqualToAnchor(&view.centerYAnchor()).setActive(true);
        ResultsUi { view, summary, review_only, kind, search, warn, excl_bar, excl_label, rescan, scroll, table, empty }
    }
}

/// 타입별 건수 (표 위 요약)
fn counts(scan: &Scan) -> Vec<(String, usize)> {
    let mut m: BTreeMap<&str, usize> = Default::default();
    for r in scan.all_records() {
        *m.entry(r.base()).or_default() += 1;
    }
    let mut v: Vec<_> = m.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

/// 같은 (원본 값, 정보타입) 끼리 묶음 — 처음 나온 순서 유지
fn group(scan: &Scan, hits: &[(Src, usize)]) -> Vec<Vec<(Src, usize)>> {
    let mut at: HashMap<(&str, &str), usize> = HashMap::new();
    let mut out: Vec<Vec<(Src, usize)>> = Vec::new();
    for &(s, k) in hits {
        let r = &scan.records(s)[k];
        let i = *at.entry((r.original.as_str(), r.base())).or_insert_with(|| {
            out.push(Vec::new());
            out.len() - 1
        });
        out[i].push((s, k));
    }
    out
}

fn group_key(scan: &Scan, g: &[(Src, usize)]) -> (String, String) {
    let r = &scan.records(g[0].0)[g[0].1];
    (r.original.clone(), r.base().to_string())
}

/// 펼친 묶음만 하위 행을 보인다
fn visible(scan: &Scan, groups: &[Vec<(Src, usize)>], open: &HashSet<(String, String)>) -> Vec<Row> {
    let mut rows = Vec::new();
    for (g, grp) in groups.iter().enumerate() {
        rows.push(Row::Group(g));
        if grp.len() > 1 && open.contains(&group_key(scan, grp)) {
            rows.extend((0..grp.len()).map(|m| Row::Member(g, m)));
        }
    }
    rows
}

/// 검사 결과·필터가 바뀌면 다시 계산
pub fn refresh() {
    let u = &ui().res;
    let review_only = is_on(&u.review_only);
    let kind = (u.kind.indexOfSelectedItem() > 0).then(|| u.kind.titleOfSelectedItem().map(|s| s.to_string())).flatten();
    let q = kit::text(&u.search).trim().to_lowercase();
    let (summary, kinds, review) = st_mut(|s| {
        let Some(scan) = s.scan.as_ref().filter(|x| x.all_records().next().is_some()) else {
            s.res.groups.clear();
            s.res.rows.clear();
            return (None, vec![], 0);
        };
        let total = scan.all_records().count();
        let review = scan.all_records().filter(|r| r.review).count();
        let kinds = counts(scan);
        let mut line = format!("총 {total}건");
        if review > 0 {
            line.push_str(&format!("   검토필요 {review}건"));
        }
        for (k, n) in &kinds {
            line.push_str(&format!("   · {k} {n}"));
        }
        let srcs = (0..scan.names.len()).map(Src::Name).chain((0..scan.entries.len()).map(Src::File));
        let mut hits = Vec::new();
        for src in srcs {
            let rel = scan.rel(scan.path(src)).to_lowercase();
            for (k, r) in scan.records(src).iter().enumerate() {
                let ok = (!review_only || r.review)
                    && kind.as_ref().map_or(true, |t| r.base() == t.as_str())
                    && (q.is_empty() || r.original.to_lowercase().contains(&q) || rel.contains(&q));
                if ok {
                    hits.push((src, k));
                }
            }
        }
        let names: Vec<String> = kinds.iter().map(|(k, _)| k.clone()).collect();
        s.res.groups = group(scan, &hits);
        s.res.rows = visible(scan, &s.res.groups, &s.res.open);
        (Some(line), names, review)
    });
    let has = summary.is_some();
    u.summary.setStringValue(&kit::ns(&summary.unwrap_or_default()));
    u.warn.setHidden(review == 0);
    for v in [&*u.summary as &NSView, &u.review_only, &u.kind, &u.search, &u.scroll] {
        v.setHidden(!has);
    }
    u.empty.setHidden(has);
    // 타입 목록이 바뀐 경우만 다시 채운다 (선택 유지)
    let cur: Vec<String> = (1..u.kind.numberOfItems()).map(|i| u.kind.itemTitleAtIndex(i).to_string()).collect();
    if cur != kinds {
        let sel = u.kind.titleOfSelectedItem().map(|s| s.to_string());
        u.kind.removeAllItems();
        u.kind.addItemWithTitle(&kit::ns("모든 타입"));
        for k in &kinds {
            u.kind.addItemWithTitle(&kit::ns(k));
        }
        if let Some(s) = sel {
            u.kind.selectItemWithTitle(&kit::ns(&s));
        }
    }
    refresh_excl();
    u.table.reloadData();
}

/// 제외 상태 줄 (제외가 있거나 반영된 제외가 있을 때만 보임)
pub fn refresh_excl() {
    let u = &ui().res;
    let (text, show, pending, running) = st(|s| {
        let e = &s.excl;
        let (nv, nf) = (e.values.len(), e.formats.len());
        let show = nv + nf > 0 || !e.applied.0.is_empty() || !e.applied.1.is_empty();
        let what = format!("제외: 값 {nv}개 · 형식 {nf}개");
        let text = if e.pending() { format!("{what} — 다시 검사해야 결과 파일에 반영됩니다") } else { format!("{what} — 결과 파일에 반영됨 (제외한 값은 검출 목록에 나오지 않음)") };
        (text, show, e.pending(), s.job.is_some())
    });
    u.excl_label.setStringValue(&kit::ns(&text));
    let color = if pending { kit::warn_color() } else { objc2_app_kit::NSColor::secondaryLabelColor() };
    u.excl_label.setTextColor(Some(&color));
    u.excl_bar.setHidden(!show);
    u.rescan.setEnabled(pending && !running);
}

pub fn count() -> isize {
    st(|s| s.res.rows.len() as isize)
}

/// 행이 가리키는 검출 (묶음 행은 첫 건)과 묶음 크기 (펼친 하위 행은 0)
fn row_rec(s: &super::State, row: usize) -> Option<((Src, usize), usize)> {
    match *s.res.rows.get(row)? {
        Row::Group(g) => {
            let grp = s.res.groups.get(g)?;
            Some((*grp.first()?, grp.len()))
        }
        Row::Member(g, m) => Some((*s.res.groups.get(g)?.get(m)?, 0)),
    }
}

pub fn view_for(tv: &NSTableView, col: Option<&NSTableColumn>, row: usize) -> Option<Retained<NSView>> {
    let id = col_id(col);
    if id == "chk" {
        let on = st(|s| {
            let ((src, k), _) = row_rec(s, row)?;
            let r = s.scan.as_ref()?.records(src).get(k)?;
            Some(s.excl.reason(r).is_none())
        })?;
        let cid = kit::ns("reschk");
        let b = unsafe { tv.makeViewWithIdentifier_owner(&cid, None) }.and_then(|v| v.downcast::<NSButton>().ok()).unwrap_or_else(|| {
            let b = checkbox("", &*ui().ctrl, sel!(resCheck:));
            b.setIdentifier(Some(&cid));
            b.setToolTip(Some(&kit::ns("해제하면 이 값(같은 값 모두)은 치환하지 않습니다")));
            b
        });
        b.setTag(row as isize);
        set_on(&b, on);
        return Some(Retained::into_super(Retained::into_super(b)));
    }
    let (text, look) = st(|s| {
        let scan = s.scan.as_ref()?;
        let ((src, k), n) = row_rec(s, row)?;
        let r = scan.records(src).get(k)?;
        let member = matches!(s.res.rows.get(row), Some(Row::Member(..)));
        let warn = r.review;
        Some(match id.as_str() {
            "orig" if member => (format!("    └ {}", r.original), Look { mono: true, dim: true, ..PLAIN }),
            "orig" if n > 1 => (format!("{}  ({n}건)", r.original), Look { mono: true, warn, ..PLAIN }),
            "orig" => (r.original.clone(), Look { mono: true, warn, ..PLAIN }),
            "masked" => match s.excl.reason(r) {
                Some(why) => (why.to_string(), Look { dim: true, ..PLAIN }),
                None => (r.masked.clone(), Look { mono: true, ..PLAIN }),
            },
            "label" => (r.label.clone(), Look { warn, ..PLAIN }),
            _ if n > 1 => {
                // 묶음: 모두 같은 파일이면 그 파일, 아니면 파일 수
                let Some(Row::Group(g)) = s.res.rows.get(row) else { return None };
                let mut files: Vec<&std::path::Path> = s.res.groups[*g].iter().map(|(x, _)| scan.path(*x)).collect();
                files.dedup();
                if files.len() == 1 {
                    (scan.rel(files[0]), PLAIN)
                } else {
                    (format!("파일 {}개 (두 번 눌러 펼치기)", files.len()), Look { dim: true, ..PLAIN })
                }
            }
            _ => (scan.rel(scan.path(src)), PLAIN),
        })
    })?;
    Some(cell(tv, col, &text, look, None))
}

/// 우클릭 대상 행들: 누른 행이 선택에 들어 있으면 선택한 행 모두, 아니면 누른 행만
fn target_rows() -> Vec<usize> {
    let t = &ui().res.table;
    let Some(clicked) = kit::row(t.clickedRow()) else { return vec![] };
    let sel = t.selectedRowIndexes();
    if !sel.containsIndex(clicked) {
        return vec![clicked];
    }
    let mut v = Vec::new();
    let mut i = sel.firstIndex();
    while i != objc2_foundation::NSNotFound as usize {
        v.push(i);
        i = sel.indexGreaterThanIndex(i);
    }
    v
}

/// 행들의 검출 레코드 (원본 값·형식 키)
fn recs_of(rows: &[usize]) -> Vec<(String, String)> {
    st(|s| {
        let Some(scan) = s.scan.as_ref() else { return vec![] };
        rows.iter()
            .filter_map(|&row| row_rec(s, row))
            .map(|((src, k), _)| {
                let r = &scan.records(src)[k];
                (r.original.clone(), Excl::format_of(r))
            })
            .collect()
    })
}

fn changed() {
    refresh_excl();
    ui().res.table.reloadData();
}

/// 체크 상자: 제외돼 있으면 되살리고(값·형식 모두), 아니면 그 값을 제외
pub fn toggle(b: &NSButton) {
    let recs = recs_of(&[b.tag() as usize]);
    st_mut(|s| {
        for (v, f) in recs {
            if s.excl.values.contains(&v) || s.excl.formats.contains(&f) {
                s.excl.values.remove(&v);
                s.excl.formats.remove(&f);
            } else {
                s.excl.values.insert(v);
            }
        }
    });
    changed();
}

pub fn exclude() {
    let recs = recs_of(&target_rows());
    st_mut(|s| s.excl.values.extend(recs.into_iter().map(|(v, _)| v)));
    changed();
}

pub fn exclude_format() {
    let recs = recs_of(&target_rows());
    st_mut(|s| s.excl.formats.extend(recs.into_iter().map(|(_, f)| f)));
    changed();
}

pub fn include() {
    let recs = recs_of(&target_rows());
    st_mut(|s| {
        for (v, f) in recs {
            s.excl.values.remove(&v);
            s.excl.formats.remove(&f);
        }
    });
    changed();
}

pub fn clear_excl() {
    st_mut(|s| {
        s.excl.values.clear();
        s.excl.formats.clear();
    });
    changed();
}

/// 묶음 행 두 번 누름: 펼치기/접기
pub fn double_click() {
    let Some(row) = kit::row(ui().res.table.clickedRow()) else { return };
    st_mut(|s| {
        let (Some(scan), Some(Row::Group(g))) = (s.scan.as_ref(), s.res.rows.get(row).copied()) else { return };
        let grp = &s.res.groups[g];
        if grp.len() < 2 {
            return;
        }
        let k = group_key(scan, grp);
        if !s.res.open.remove(&k) {
            s.res.open.insert(k);
        }
        s.res.rows = visible(scan, &s.res.groups, &s.res.open);
    });
    ui().res.table.reloadData();
}

/// 우클릭한 행의 검출 1건
fn clicked() -> Option<(Src, usize)> {
    let r = kit::row(ui().res.table.clickedRow())?;
    st(|s| row_rec(s, r).map(|(x, _)| x))
}

pub fn reveal_src() {
    if let Some(p) = clicked().and_then(|(src, _)| st(|s| s.scan.as_ref().map(|x| x.path(src).to_path_buf()))) {
        sys::reveal(&p);
    }
}

pub fn reveal_out() {
    if let Some(p) = clicked().and_then(|(src, _)| st(|s| s.scan.as_ref().and_then(|x| x.out_path(src)).map(|p| p.to_path_buf()))) {
        sys::reveal(&p);
    }
}

pub fn copy() {
    if let Some(t) = clicked().and_then(|(src, k)| st(|s| s.scan.as_ref().map(|x| x.records(src)[k].original.clone()))) {
        kit::copy_text(&t);
    }
}

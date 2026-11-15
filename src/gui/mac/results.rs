//! 검출 결과 탭 — 보고서(CSV)와 같은 내용을 사람이 검토하는 표. 검토필요 행은 주황 글자.

use super::kit::{self, cell, checkbox, col_id, is_on, label, note, Look, H, PLAIN};
use super::{st, st_mut, ui, Controller};
use crate::gui::scan::{Scan, Src};
use crate::gui::sys;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{sel, MainThreadOnly};
use objc2_app_kit::{NSButton, NSMenu, NSMenuItem, NSPopUpButton, NSScrollView, NSSearchField, NSTableColumn, NSTableView, NSTextField, NSView};
use std::collections::BTreeMap;

pub struct ResultsUi {
    pub view: Retained<NSView>,
    summary: Retained<NSTextField>,
    review_only: Retained<NSButton>,
    kind: Retained<NSPopUpButton>,
    search: Retained<NSSearchField>,
    warn: Retained<NSTextField>,
    scroll: Retained<NSScrollView>,
    pub table: Retained<NSTableView>,
    empty: Retained<NSTextField>,
}

#[derive(Default)]
pub struct ResState {
    /// 보이는 (위치, 번호)
    rows: Vec<(Src, usize)>,
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
        let warn = note("※ 애매한 항목도 일단 치환했습니다. 주황색 행을 확인하고, 정보가 아니면 원본에서 해당 부분을 되살리세요.");
        warn.setTextColor(Some(&kit::warn_color()));
        let (scroll, table) = kit::table(&[("orig", "검출된정보", 170.0), ("masked", "치환정보", 170.0), ("label", "정보타입", 130.0), ("path", "파일경로", 300.0)]);
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(ctrl)));
            table.setDelegate(Some(ProtocolObject::from_ref(ctrl)));
        }
        let menu = NSMenu::new(kit::mtm());
        menu.addItem(&menu_item("원본 파일 위치 열기", sel!(resRevealSrc:), t));
        menu.addItem(&menu_item("결과 파일 위치 열기", sel!(resRevealOut:), t));
        menu.addItem(&menu_item("검출된 정보 복사", sel!(resCopy:), t));
        unsafe { table.setMenu(Some(&menu)) };
        let empty = label("검사를 마치면 검출된 정보가 여기에 표시됩니다. 보고서(CSV)와 같은 내용입니다.");
        empty.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
        let view = kit::container();
        kit::column(&view, &[(&summary, H::Auto), (&filters, H::Auto), (&warn, H::Auto), (&scroll, H::Fill)], 8.0, 6.0);
        empty.setTranslatesAutoresizingMaskIntoConstraints(false);
        view.addSubview(&empty);
        empty.centerXAnchor().constraintEqualToAnchor(&view.centerXAnchor()).setActive(true);
        empty.centerYAnchor().constraintEqualToAnchor(&view.centerYAnchor()).setActive(true);
        ResultsUi { view, summary, review_only, kind, search, warn, scroll, table, empty }
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

/// 검사 결과·필터가 바뀌면 다시 계산
pub fn refresh() {
    let u = &ui().res;
    let review_only = is_on(&u.review_only);
    let kind = (u.kind.indexOfSelectedItem() > 0).then(|| u.kind.titleOfSelectedItem().map(|s| s.to_string())).flatten();
    let q = kit::text(&u.search).trim().to_lowercase();
    let (summary, kinds, review) = st_mut(|s| {
        let Some(scan) = s.scan.as_ref().filter(|x| x.all_records().next().is_some()) else {
            s.res = ResState::default();
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
        let mut rows = Vec::new();
        for src in srcs {
            let rel = scan.rel(scan.path(src)).to_lowercase();
            for (k, r) in scan.records(src).iter().enumerate() {
                let ok = (!review_only || r.review)
                    && kind.as_ref().map_or(true, |t| r.base() == t.as_str())
                    && (q.is_empty() || r.original.to_lowercase().contains(&q) || rel.contains(&q));
                if ok {
                    rows.push((src, k));
                }
            }
        }
        let names: Vec<String> = kinds.iter().map(|(k, _)| k.clone()).collect();
        s.res = ResState { rows };
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
    u.table.reloadData();
}

pub fn count() -> isize {
    st(|s| s.res.rows.len() as isize)
}

pub fn view_for(tv: &NSTableView, col: Option<&NSTableColumn>, row: usize) -> Option<Retained<NSView>> {
    let (text, look) = st(|s| {
        let scan = s.scan.as_ref()?;
        let (src, k) = *s.res.rows.get(row)?;
        let r = scan.records(src).get(k)?;
        let warn = r.review;
        Some(match col_id(col).as_str() {
            "orig" => (r.original.clone(), Look { mono: true, warn, ..PLAIN }),
            "masked" => (r.masked.clone(), Look { mono: true, ..PLAIN }),
            "label" => (r.label.clone(), Look { warn, ..PLAIN }),
            _ => (scan.rel(scan.path(src)), PLAIN),
        })
    })?;
    Some(cell(tv, col, &text, look, None))
}

/// 우클릭한 행의 검출 1건
fn clicked() -> Option<(Src, usize)> {
    let r = kit::row(ui().res.table.clickedRow())?;
    st(|s| s.res.rows.get(r).copied())
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

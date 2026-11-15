//! 확장자 선택 창 — 동영상 플레이어의 "지원 확장자 선택" 같은 분류별 체크 목록.

use super::kit::{self, button, cell, checkbox, col_id, is_on, note, set_on, Look, H, PLAIN};
use super::{running, st, st_mut, ui, Controller};
use crate::exts::{self, ExtFilter, ExtInfo};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{sel, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSPopUpButton, NSUserInterfaceItemIdentification, NSSearchField, NSTableColumn, NSTableView, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};

pub struct ExtUi {
    pub window: Retained<NSWindow>,
    category: Retained<NSPopUpButton>,
    search: Retained<NSSearchField>,
    pub table: Retained<NSTableView>,
    unknown: Retained<NSButton>,
    disguised: Retained<NSButton>,
    count: Retained<NSTextField>,
    buttons: Vec<Retained<NSButton>>,
}

pub enum Row {
    Cat(&'static str),
    Ext(&'static ExtInfo),
}

#[derive(Default)]
pub struct ExtState {
    rows: Vec<Row>,
}

pub fn new_window(title: &str, w: f64, h: f64) -> Retained<NSWindow> {
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable | NSWindowStyleMask::Miniaturizable;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h));
    let win = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(kit::mtm()), rect, style, NSBackingStoreType::Buffered, false) };
    unsafe { win.setReleasedWhenClosed(false) };
    win.setTitle(&kit::ns(title));
    win
}

impl ExtUi {
    pub fn build(ctrl: &Controller) -> ExtUi {
        let t: &AnyObject = ctrl.as_ref();
        let window = new_window("검사할 확장자 선택", 600.0, 520.0);
        window.setContentMinSize(NSSize::new(440.0, 320.0));
        let desc = note("체크한 확장자만 검사합니다. 실제 형식은 항상 파일 내용(매직바이트)으로 판별하며, 체크하지 않은 확장자는 원본 그대로 복사됩니다.");
        let category = NSPopUpButton::new(kit::mtm());
        category.addItemWithTitle(&kit::ns("전체 분류"));
        for c in exts::categories() {
            category.addItemWithTitle(&kit::ns(c));
        }
        unsafe {
            category.setTarget(Some(t));
            category.setAction(Some(sel!(extFilter:)));
        }
        let search = kit::search("확장자·설명 검색", t, sel!(extFilter:));
        let filters = kit::hstack(&[&category, &search, &kit::spacer()]);
        let buttons = vec![button("보이는 항목 선택", t, sel!(extSelectShown:)), button("보이는 항목 해제", t, sel!(extClearShown:)), button("기본값", t, sel!(extDefaults:))];
        let sp = kit::spacer();
        let mut row: Vec<&NSView> = buttons.iter().map(|b| -> &NSView { b }).collect();
        row.push(&sp);
        let btn_row = kit::hstack(&row);
        let (scroll, table) = kit::table(&[("ext", "확장자", 120.0), ("desc", "설명", 380.0)]);
        table.setUsesAlternatingRowBackgroundColors(false);
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(ctrl)));
            table.setDelegate(Some(ProtocolObject::from_ref(ctrl)));
        }
        let unknown = checkbox("목록에 없는 확장자와 확장자 없는 파일도 검사 (매직바이트로 판별)", t, sel!(extFlags:));
        let disguised = checkbox("확장자를 위장한 파일은 체크하지 않았어도 검사 (예: 이름만 .log 인 Word 문서)", t, sel!(extFlags:));
        let count = kit::label("");
        let content = window.contentView().expect("창 내용");
        kit::column(
            &content,
            &[(&desc, H::Auto), (&filters, H::Auto), (&btn_row, H::Auto), (&scroll, H::Fill), (&unknown, H::Auto), (&disguised, H::Auto), (&count, H::Auto)],
            12.0,
            8.0,
        );
        window.center();
        ExtUi { window, category, search, table, unknown, disguised, count, buttons }
    }
}

/// 분류·검색에 맞는 행 다시 계산
pub fn refresh() {
    let u = &ui().ext;
    let cat = (u.category.indexOfSelectedItem() > 0).then(|| u.category.titleOfSelectedItem().map(|s| s.to_string())).flatten();
    let q = kit::text(&u.search).trim().to_lowercase();
    let (n_on, unknown, disguised) = st_mut(|s| {
        let shown: Vec<&'static ExtInfo> = exts::catalog()
            .iter()
            .filter(|e| cat.as_deref().map_or(true, |c| e.category == c))
            .filter(|e| q.is_empty() || e.ext.contains(q.as_str()) || e.desc.to_lowercase().contains(q.as_str()))
            .collect();
        let mut rows = Vec::new();
        for c in exts::categories() {
            let items: Vec<&&ExtInfo> = shown.iter().filter(|e| e.category == c).collect();
            if items.is_empty() {
                continue;
            }
            rows.push(Row::Cat(c));
            rows.extend(items.into_iter().map(|e| Row::Ext(e)));
        }
        s.ext.rows = rows;
        (s.exts.enabled.len(), s.exts.scan_unknown, s.exts.check_disguised)
    });
    let on = !running();
    set_on(&u.unknown, unknown);
    set_on(&u.disguised, disguised);
    for c in [&*u.unknown, &*u.disguised].into_iter().chain(u.buttons.iter().map(|b| &**b)) {
        c.setEnabled(on);
    }
    u.count.setStringValue(&kit::ns(&format!("선택 {n_on} / {}", exts::catalog().len())));
    u.table.reloadData();
    super::options_changed();
}

pub fn show() {
    refresh();
    ui().ext.window.makeKeyAndOrderFront(None);
}

pub fn count() -> isize {
    st(|s| s.ext.rows.len() as isize)
}

pub fn is_group(row: usize) -> bool {
    st(|s| matches!(s.ext.rows.get(row), Some(Row::Cat(_))))
}

pub fn view_for(tv: &NSTableView, col: Option<&NSTableColumn>, row: usize) -> Option<Retained<NSView>> {
    enum V {
        Cat(String),
        Check(&'static ExtInfo, bool),
        Desc(&'static str),
    }
    let v = st(|s| {
        Some(match s.ext.rows.get(row)? {
            Row::Cat(c) => {
                let all: Vec<&ExtInfo> = s.ext.rows.iter().filter_map(|r| if let Row::Ext(e) = r { (e.category == *c).then_some(*e) } else { None }).collect();
                let n_on = all.iter().filter(|e| s.exts.enabled.contains(e.ext)).count();
                V::Cat(format!("{c}  ({n_on}/{})", all.len()))
            }
            Row::Ext(e) if col_id(col) == "ext" => V::Check(e, s.exts.enabled.contains(e.ext)),
            Row::Ext(e) => V::Desc(e.desc),
        })
    })?;
    Some(match v {
        V::Cat(t) => cell(tv, None, &t, Look { bold: true, ..PLAIN }, None),
        V::Desc(d) => cell(tv, col, d, Look { dim: true, ..PLAIN }, None),
        V::Check(e, on) => {
            let id = kit::ns("extchk");
            let b = unsafe { tv.makeViewWithIdentifier_owner(&id, None) }.and_then(|v| v.downcast::<NSButton>().ok()).unwrap_or_else(|| {
                let b = checkbox("", &*ui().ctrl, sel!(extToggle:));
                b.setIdentifier(Some(&id));
                b
            });
            b.setTitle(&kit::ns(&format!(".{}", e.ext)));
            b.setToolTip(Some(&kit::ns(e.desc)));
            b.setTag(row as isize);
            set_on(&b, on);
            b.setEnabled(!running());
            b.into_super().into_super()
        }
    })
}

fn set_shown(on: bool) {
    st_mut(|s| {
        let shown: Vec<&'static str> = s.ext.rows.iter().filter_map(|r| if let Row::Ext(e) = r { Some(e.ext) } else { None }).collect();
        for e in shown {
            if on {
                s.exts.enabled.insert(e.to_string());
            } else {
                s.exts.enabled.remove(e);
            }
        }
    });
    refresh();
}

pub fn select_shown() {
    set_shown(true);
}

pub fn clear_shown() {
    set_shown(false);
}

pub fn defaults() {
    st_mut(|s| s.exts.enabled = ExtFilter::default().enabled);
    refresh();
}

pub fn toggle(b: &NSButton) {
    let row = b.tag() as usize;
    let on = is_on(b);
    st_mut(|s| {
        if let Some(Row::Ext(e)) = s.ext.rows.get(row) {
            if on {
                s.exts.enabled.insert(e.ext.to_string());
            } else {
                s.exts.enabled.remove(e.ext);
            }
        }
    });
    refresh();
}

pub fn flags() {
    let u = &ui().ext;
    let (a, b) = (is_on(&u.unknown), is_on(&u.disguised));
    st_mut(|s| {
        s.exts.scan_unknown = a;
        s.exts.check_disguised = b;
    });
    refresh();
}

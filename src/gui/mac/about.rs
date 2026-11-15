//! 정보 창 — 프로그램·개발자 정보와 사용한 오픈소스 목록 (두 번 클릭: 저장소 열기).

use super::ext_window::new_window;
use super::kit::{self, button, cell, col_id, label, note, Look, H, PLAIN};
use super::{st, st_mut, ui, Controller};
use crate::gui::about::{crates, GITHUB, RUNTIME};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::sel;
use objc2_app_kit::{NSSearchField, NSTableColumn, NSTableView, NSView, NSWindow, NSWorkspace};
use objc2_foundation::{NSSize, NSURL};

pub struct AboutUi {
    pub window: Retained<NSWindow>,
    search: Retained<NSSearchField>,
    pub table: Retained<NSTableView>,
}

enum Row {
    Crate([&'static str; 5]),
    Group(&'static str),
    Runtime(&'static (&'static str, &'static str, &'static str)),
}

#[derive(Default)]
pub struct AboutState {
    rows: Vec<Row>,
}

impl AboutUi {
    pub fn build(ctrl: &Controller) -> AboutUi {
        let t: &AnyObject = ctrl.as_ref();
        let window = new_window("deid 정보", 620.0, 560.0);
        window.setContentMinSize(NSSize::new(460.0, 360.0));
        let title = kit::label(&format!("deid   버전 {}", env!("CARGO_PKG_VERSION")));
        title.setFont(Some(&kit::font(20.0, true)));
        let desc = label("파일 속 개인정보·민감정보 비식별화 도구 — 순수 Rust, 단일 실행 파일");
        let dev = kit::hstack(&[&kit::small_label("개발자"), &button("github.com/maspo1", t, sel!(openGithub:)), &kit::spacer()]);
        let gui = note("화면: AppKit (macOS 네이티브)\n검사 엔진: 별도 프로세스 (검사할 때만 실행, 끝나면 메모리 반환)");
        let n = crates().count();
        let search = kit::search("이름·라이선스 검색", t, sel!(aboutFilter:));
        let head = kit::hstack(&[&kit::bold_label(&format!("사용한 오픈소스 ({n}개)")), &kit::spacer(), &search]);
        let (scroll, table) = kit::table(&[("name", "이름", 190.0), ("ver", "버전", 70.0), ("lic", "라이선스", 170.0), ("usage", "용도", 140.0)]);
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(ctrl)));
            table.setDelegate(Some(ProtocolObject::from_ref(ctrl)));
            table.setTarget(Some(t));
            table.setDoubleAction(Some(sel!(aboutOpen:)));
        }
        let content = window.contentView().expect("창 내용");
        kit::column(&content, &[(&title, H::Auto), (&desc, H::Auto), (&dev, H::Auto), (&gui, H::Auto), (&head, H::Auto), (&scroll, H::Fill)], 14.0, 8.0);
        window.center();
        AboutUi { window, search, table }
    }
}

pub fn refresh() {
    let q = kit::text(&ui().about.search).trim().to_lowercase();
    st_mut(|s| {
        let mut rows: Vec<Row> = crates().filter(|c| q.is_empty() || c[0].contains(&q) || c[2].to_lowercase().contains(&q)).map(Row::Crate).collect();
        rows.push(Row::Group("실행 시 사용하는 OS 기능·글꼴"));
        rows.extend(RUNTIME.iter().map(Row::Runtime));
        s.about.rows = rows;
    });
    ui().about.table.reloadData();
}

pub fn show() {
    refresh();
    ui().about.window.makeKeyAndOrderFront(None);
}

pub fn count() -> isize {
    st(|s| s.about.rows.len() as isize)
}

pub fn is_group(row: usize) -> bool {
    st(|s| matches!(s.about.rows.get(row), Some(Row::Group(_))))
}

pub fn view_for(tv: &NSTableView, col: Option<&NSTableColumn>, row: usize) -> Option<Retained<NSView>> {
    let id = col_id(col);
    let (text, look) = st(|s| {
        Some(match s.about.rows.get(row)? {
            Row::Group(g) => (g.to_string(), Look { bold: true, ..PLAIN }),
            Row::Crate([name, ver, lic, _, usage]) => match id.as_str() {
                "name" => (name.to_string(), PLAIN),
                "ver" => (ver.to_string(), PLAIN),
                "lic" => (lic.to_string(), PLAIN),
                _ => (usage.to_string(), Look { dim: true, ..PLAIN }),
            },
            Row::Runtime((a, b, c)) => match id.as_str() {
                "name" => (a.to_string(), PLAIN),
                "lic" => (c.to_string(), Look { dim: true, ..PLAIN }),
                "usage" => (b.to_string(), PLAIN),
                _ => (String::new(), PLAIN),
            },
        })
    })?;
    Some(cell(tv, col, &text, look, None))
}

pub fn open_url(url: &str) {
    if let Some(u) = NSURL::URLWithString(&kit::ns(url)) {
        NSWorkspace::sharedWorkspace().openURL(&u);
    }
}

pub fn open_clicked() {
    let Some(r) = kit::row(ui().about.table.clickedRow()) else { return };
    let url = st(|s| match s.about.rows.get(r) {
        Some(Row::Crate(c)) if !c[3].is_empty() => Some(c[3].to_string()),
        _ => None,
    });
    if let Some(u) = url {
        open_url(&u);
    }
}

pub fn open_github() {
    open_url(GITHUB);
}

//! 가운데 탭 중 로그·미리보기. 검출 결과는 `results.rs`, 파일 분석은 `forensic.rs`.

use super::kit::{self, cell, col_id, note, Look, H, PLAIN};
use super::{st, st_mut, ui, Controller};
use crate::common::mask_text;
use crate::rules::{Detector, Lang};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::{NSColor, NSFontAttributeName, NSForegroundColorAttributeName, NSScrollView, NSTableColumn, NSTableView, NSTextField, NSTextView, NSView};
use objc2_foundation::{NSAttributedString, NSDictionary, NSRange};

// ───────────────────────────── 로그 ─────────────────────────────

pub struct LogUi {
    pub scroll: Retained<NSScrollView>,
    tv: Retained<NSTextView>,
}

impl LogUi {
    pub fn build() -> LogUi {
        let (scroll, tv) = kit::text_view(false);
        scroll.setBorderType(objc2_app_kit::NSBorderType::NoBorder);
        LogUi { scroll, tv }
    }

    pub fn clear(&self) {
        self.tv.setString(&kit::ns(""));
    }

    /// 새 줄 덧붙이고 맨 아래로
    pub fn append(&self, lines: &[String]) {
        if lines.is_empty() {
            return;
        }
        let mut t = lines.join("\n");
        t.push('\n');
        let font = kit::mono(12.0);
        let color = NSColor::textColor();
        let attrs = NSDictionary::<objc2_foundation::NSAttributedStringKey, AnyObject>::from_slices(
            unsafe { &[NSFontAttributeName, NSForegroundColorAttributeName] },
            &[font.as_ref(), color.as_ref()],
        );
        let s = unsafe { NSAttributedString::new_with_attributes(&kit::ns(&t), &attrs) };
        if let Some(store) = unsafe { self.tv.textStorage() } {
            store.appendAttributedString(&s);
            let len = store.length();
            self.tv.scrollRangeToVisible(NSRange::new(len, 0));
        }
    }
}

// ───────────────────────────── 미리보기 ─────────────────────────────

/// 현재 설정으로 문장을 바로 검사해 본다 (사용자 정의 정규식 시험용)
pub struct PreviewUi {
    pub view: Retained<NSView>,
    pub input: Retained<NSTextView>,
    output: Retained<NSTextView>,
    count: Retained<NSTextField>,
    err: Retained<NSTextField>,
    pub table: Retained<NSTableView>,
}

impl PreviewUi {
    pub fn build(ctrl: &Controller) -> PreviewUi {
        let desc = note("현재 설정(검출 항목·맥락 탐지·사용자 정의 정규식)으로 문장을 바로 검사합니다. 파일은 바뀌지 않습니다.\n예) 담당자 010-1234-5678, server 192-168-0-1, password=hunter2");
        let (in_scroll, input) = kit::text_view(true);
        input.setDelegate(Some(ProtocolObject::from_ref(ctrl)));
        let out_head = kit::bold_label("치환 결과");
        let (out_scroll, output) = kit::text_view(false);
        let count = kit::bold_label("검출 0건");
        let err = note("");
        err.setTextColor(Some(&NSColor::systemRedColor()));
        let (scroll, table) = kit::table(&[("orig", "검출된정보", 200.0), ("masked", "치환정보", 200.0), ("label", "정보타입", 160.0)]);
        unsafe {
            table.setDataSource(Some(ProtocolObject::from_ref(ctrl)));
            table.setDelegate(Some(ProtocolObject::from_ref(ctrl)));
        }
        let view = kit::container();
        kit::column(
            &view,
            &[(&desc, H::Auto), (&in_scroll, H::Fixed(110.0)), (&out_head, H::Auto), (&out_scroll, H::Fixed(80.0)), (&count, H::Auto), (&err, H::Auto), (&scroll, H::Fill)],
            8.0,
            6.0,
        );
        PreviewUi { view, input, output, count, err, table }
    }
}

/// 입력·설정이 바뀌면 다시 검사
pub fn preview_refresh() {
    let u = &ui().prev;
    let text = kit::tv_text(&u.input);
    let r = super::settings().options().map(|o| {
        let det = Detector::new(o);
        let (masked, recs) = mask_text(&text, &det.scan(&text, Lang::None));
        (masked, recs.iter().map(|r| (r.original.clone(), r.masked.clone(), r.label())).collect::<Vec<_>>())
    });
    let (masked, recs, err) = match r {
        Ok((m, recs)) => (m, recs, String::new()),
        Err(e) => (String::new(), vec![], e),
    };
    u.output.setString(&kit::ns(&masked));
    u.count.setStringValue(&kit::ns(&format!("검출 {}건", recs.len())));
    u.err.setStringValue(&kit::ns(&err));
    st_mut(|s| s.prev_recs = recs);
    u.table.reloadData();
}

pub fn preview_count() -> isize {
    st(|s| s.prev_recs.len() as isize)
}

pub fn preview_view(tv: &NSTableView, col: Option<&NSTableColumn>, row: usize) -> Option<Retained<NSView>> {
    let (o, m, l) = st(|s| s.prev_recs.get(row).cloned())?;
    Some(match col_id(col).as_str() {
        "orig" => cell(tv, col, &o, Look { mono: true, ..PLAIN }, None),
        "masked" => cell(tv, col, &m, Look { mono: true, ..PLAIN }, None),
        _ => cell(tv, col, &l, PLAIN, None),
    })
}

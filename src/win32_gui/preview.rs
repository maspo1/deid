//! 미리보기 탭 — 현재 설정(검출 항목·맥락 탐지·사용자 정의 정규식)으로 문장을 바로 검사. 파일은 바뀌지 않는다.

use super::listview::{self, col, Col};
use super::{ctl, id, layout, App, FontKind};
use crate::common::mask_text;
use crate::rules::{Detector, Lang};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{EN_CHANGE, WS_VSCROLL};

const COLS: [Col; 3] = [col("검출된정보", 220), col("치환정보", 220), col("정보타입", 0)];

#[derive(Default)]
pub struct Page {
    help: HWND,
    input: HWND,
    out_label: HWND,
    output: HWND,
    count: HWND,
    list: HWND,
    /// (입력, 설정 지문) — 같으면 다시 계산하지 않음
    last: Option<(String, String)>,
}

impl App {
    pub(super) fn preview_create(&mut self) {
        let (h, dpi) = (self.hwnd, self.dpi());
        let reg = &mut self.reg;
        let p = &mut self.pv;
        let multi = 0x4 | 0x1000 | 0x40 | WS_VSCROLL.0; // ES_MULTILINE | ES_WANTRETURN | ES_AUTOVSCROLL
        p.help = ctl::label(h, "현재 설정으로 문장을 바로 검사합니다 (예: 담당자 010-1234-5678, server 192-168-0-1, password=hunter2). 파일은 바뀌지 않습니다.", reg, FontKind::Ui);
        p.input = ctl::edit(h, id::P_INPUT, multi, reg, FontKind::Mono);
        p.out_label = ctl::label(h, "치환 결과", reg, FontKind::Bold);
        p.output = ctl::edit(h, 0, multi | 0x800 /* ES_READONLY */, reg, FontKind::Mono);
        p.count = ctl::label(h, "", reg, FontKind::Bold);
        p.list = listview::create(h, id::P_LIST, false, 0, &COLS, dpi, reg, FontKind::Ui);
    }

    pub(super) fn preview_layout(&self, pg: layout::R) {
        let l = layout::preview(pg, self.dpi());
        let p = &self.pv;
        for (h, r) in [(p.help, l.help), (p.input, l.input), (p.out_label, l.out_label), (p.output, l.output), (p.count, l.count), (p.list, l.list)] {
            ctl::place(h, r);
        }
        listview::fit_columns(p.list, &COLS, l.list.w, self.dpi());
    }

    pub(super) fn preview_columns(&self) {
        self.preview_layout(self.page());
    }

    pub(super) fn preview_show(&self, on: bool) {
        let p = &self.pv;
        for h in [p.help, p.input, p.out_label, p.output, p.count, p.list] {
            ctl::show(h, on);
        }
    }

    pub(super) fn preview_command(&mut self, cid: u16, code: u32) {
        if cid == id::P_INPUT && code == EN_CHANGE {
            self.preview_update();
        }
    }

    /// 입력이나 설정이 바뀌었을 때만 다시 검사
    pub(super) fn preview_update(&mut self) {
        let text = ctl::text(self.pv.input).replace("\r\n", "\n");
        let fp = format!("{:?}{}{}{}", self.s.enabled, self.s.ml, self.s.threshold, self.s.patterns);
        if self.pv.last.as_ref() == Some(&(text.clone(), fp.clone())) {
            return;
        }
        self.pv.last = Some((text.clone(), fp));
        listview::clear(self.pv.list);
        match self.s.options() {
            Err(e) => {
                ctl::set_text(self.pv.output, &format!("설정 오류: {e}"));
                ctl::set_text(self.pv.count, "");
            }
            Ok(o) => {
                let det = Detector::new(o);
                let (masked, recs) = mask_text(&text, &det.scan(&text, Lang::None));
                ctl::set_text(self.pv.output, &masked.replace('\n', "\r\n"));
                ctl::set_text(self.pv.count, &format!("검출 {}건", recs.len()));
                for r in &recs {
                    listview::push(self.pv.list, &[&r.original, &r.masked, &r.label()]);
                }
            }
        }
    }
}

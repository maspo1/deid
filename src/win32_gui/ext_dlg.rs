//! 확장자 선택 창 — 분류·검색으로 거른 체크 목록(290종)과 예외 규칙 2개. 바꾸는 즉시 설정에 반영된다.
//! 열려 있는 동안 메인 창은 사용 중지(모달처럼 동작).

use super::ctl::{self, FontReg};
use super::dpi::{px, FontKind};
use super::layout::R;
use super::listview::{self, col, Col};
use super::{drain_acts, with_app, App};
use crate::exts::{self, ExtFilter};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const CLASS: PCWSTR = w!("deid_ext");
const COLS: [Col; 3] = [col("확장자", 90), col("분류", 170), col("설명", 0)];

const CAT: u16 = 400;
const SEARCH: u16 = 401;
const SEL_ON: u16 = 402;
const SEL_OFF: u16 = 403;
const DEFAULT: u16 = 404;
const LIST: u16 = 405;
const UNKNOWN: u16 = 406;
const DISGUISE: u16 = 407;
const OK: u16 = 408;

pub struct Dlg {
    hwnd: HWND,
    cat: HWND,
    search: HWND,
    list: HWND,
    unknown: HWND,
    disguise: HWND,
    count: HWND,
    /// 목록 행 → 카탈로그 번호
    shown: Vec<usize>,
    reg: FontReg,
}

/// 창 열기 (이미 열려 있으면 앞으로)
pub fn open(owner: HWND) {
    if let Some(Some(h)) = with_app(owner, |a| a.ext.as_ref().map(|d| d.hwnd)) {
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    let dpi = with_app(owner, |a| a.dpi()).unwrap_or(96);
    let (w, h) = (px(620, dpi), px(540, dpi));
    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(owner, &mut rc);
    }
    let (x, y) = (rc.left + (rc.right - rc.left - w) / 2, rc.top + (rc.bottom - rc.top - h) / 2);
    let title = ctl::wide("검사할 확장자 선택");
    let created = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            CLASS,
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            x,
            y,
            w,
            h,
            owner,
            None,
            ctl::hinst(),
            None,
        )
    };
    if let Ok(hwnd) = created {
        unsafe {
            let _ = EnableWindow(owner, false);
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
    }
}

pub extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
        let r = match msg {
            WM_CREATE => with_app(owner, |a| {
                a.ext_create(hwnd);
                LRESULT(0)
            }),
            WM_COMMAND => with_app(owner, |a| {
                a.ext_command((wp.0 & 0xFFFF) as u16, ((wp.0 >> 16) & 0xFFFF) as u32);
                LRESULT(0)
            }),
            WM_NOTIFY => with_app(owner, |a| a.ext_notify(lp)).flatten(),
            WM_CLOSE => {
                with_app(owner, |a| a.ext = None);
                let _ = EnableWindow(owner, true);
                let _ = DestroyWindow(hwnd);
                let _ = SetForegroundWindow(owner);
                Some(LRESULT(0))
            }
            _ => None,
        };
        drain_acts(owner);
        r.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
    }
}

impl App {
    fn ext_create(&mut self, hwnd: HWND) {
        let dpi = self.dpi();
        let p = |v| px(v, dpi);
        let mut reg = FontReg::default();
        let r = &mut reg;
        let info = ctl::label(hwnd, "체크한 확장자만 검사합니다. 실제 형식은 항상 파일 내용(매직바이트)으로 판별하며, 체크하지 않은 확장자는 원본 그대로 복사됩니다.", r, FontKind::Ui);
        let cat = ctl::child(hwnd, ctl::COMBOBOX, "", WS_TABSTOP.0 | WS_VSCROLL.0 | 3, 0, CAT, r, FontKind::Ui);
        let search = ctl::edit(hwnd, SEARCH, 0, r, FontKind::Ui);
        ctl::cue(search, "확장자·설명 검색");
        let on = ctl::button(hwnd, "보이는 항목 선택", SEL_ON, r);
        let off = ctl::button(hwnd, "보이는 항목 해제", SEL_OFF, r);
        let def = ctl::button(hwnd, "기본값", DEFAULT, r);
        let list = listview::create(hwnd, LIST, false, 0, &COLS, dpi, r, FontKind::Ui);
        let ex = LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
        ctl::send(list, LVM_SETEXTENDEDLISTVIEWSTYLE, ex as usize, ex as isize);
        let unknown = ctl::checkbox(hwnd, "목록에 없는 확장자와 확장자 없는 파일도 검사 (매직바이트로 판별)", UNKNOWN, r);
        let disguise = ctl::checkbox(hwnd, "확장자를 위장한 파일은 체크하지 않았어도 검사 (예: 이름만 .log 인 Word 문서)", DISGUISE, r);
        let count = ctl::label(hwnd, "", r, FontKind::Ui);
        let ok = ctl::child(hwnd, ctl::BUTTON, "확인", WS_TABSTOP.0 | 1, 0, OK, r, FontKind::Ui);

        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        let (cw, ch) = (rc.right, rc.bottom);
        let m = p(10);
        let fw = cw - 2 * m;
        for (h, b) in [
            (info, R::new(m, m, fw, p(34))),
            (cat, R::new(m, m + p(40), p(200), p(300))),
            (search, R::new(m + p(208), m + p(40), fw - p(208), p(24))),
            (on, R::new(m, m + p(70), p(130), p(26))),
            (off, R::new(m + p(136), m + p(70), p(130), p(26))),
            (def, R::new(m + p(272), m + p(70), p(80), p(26))),
            (list, R::new(m, m + p(102), fw, ch - m - p(102) - p(96))),
            (unknown, R::new(m, ch - m - p(90), fw, p(22))),
            (disguise, R::new(m, ch - m - p(66), fw, p(22))),
            (count, R::new(m, ch - m - p(28), fw - p(110), p(22))),
            (ok, R::new(cw - m - p(100), ch - m - p(30), p(100), p(30))),
        ] {
            ctl::place(h, b);
        }
        listview::fit_columns(list, &COLS, fw, dpi);
        reg.apply(&self.fonts);

        for t in std::iter::once("전체 분류").chain(exts::categories()) {
            let w = ctl::wide(t);
            ctl::send(cat, CB_ADDSTRING, 0, w.as_ptr() as isize);
        }
        ctl::send(cat, CB_SETCURSEL, 0, 0);
        ctl::set_check(unknown, self.s.exts.scan_unknown);
        ctl::set_check(disguise, self.s.exts.check_disguised);
        self.ext = Some(Dlg { hwnd, cat, search, list, unknown, disguise, count, shown: vec![], reg });
        self.ext_fill();
    }

    /// 분류·검색에 맞는 항목으로 목록 다시 채우기
    fn ext_fill(&mut self) {
        let Some(d) = &mut self.ext else { return };
        let cat = ctl::send(d.cat, CB_GETCURSEL, 0, 0);
        let cats = exts::categories();
        let cat = (cat > 0).then(|| cats.get(cat as usize - 1).copied()).flatten();
        let q = ctl::text(d.search).trim().to_lowercase();
        d.shown = exts::catalog()
            .iter()
            .enumerate()
            .filter(|(_, e)| cat.map_or(true, |c| e.category == c))
            .filter(|(_, e)| q.is_empty() || e.ext.contains(q.as_str()) || e.desc.to_lowercase().contains(q.as_str()))
            .map(|(i, _)| i)
            .collect();
        listview::clear(d.list);
        for &i in &d.shown {
            let e = &exts::catalog()[i];
            let row = listview::push(d.list, &[&format!(".{}", e.ext), e.category, e.desc]);
            listview::set_checked(d.list, row, self.s.exts.enabled.contains(e.ext));
        }
        self.ext_count();
    }

    fn ext_count(&self) {
        if let Some(d) = &self.ext {
            ctl::set_text(d.count, &format!("선택 {} / {}", self.s.exts.enabled.len(), exts::catalog().len()));
        }
        if let Some(p) = &self.panel {
            p.refresh_labels(&self.s);
        }
    }

    fn ext_command(&mut self, cid: u16, code: u32) {
        let Some(d) = &self.ext else { return };
        let hwnd = d.hwnd;
        match (cid, code) {
            (CAT, CBN_SELCHANGE) | (SEARCH, EN_CHANGE) => self.ext_fill(),
            (SEL_ON | SEL_OFF, BN_CLICKED) => {
                for &i in &d.shown {
                    let e = exts::catalog()[i].ext;
                    if cid == SEL_ON {
                        self.s.exts.enabled.insert(e.to_string());
                    } else {
                        self.s.exts.enabled.remove(e);
                    }
                }
                self.ext_fill();
            }
            (DEFAULT, BN_CLICKED) => {
                self.s.exts.enabled = ExtFilter::default().enabled;
                self.ext_fill();
            }
            (UNKNOWN, BN_CLICKED) => self.s.exts.scan_unknown = ctl::checked(d.unknown),
            (DISGUISE, BN_CLICKED) => self.s.exts.check_disguised = ctl::checked(d.disguise),
            (OK, BN_CLICKED) | (2 /* IDCANCEL: Esc */, _) => unsafe {
                let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
            },
            _ => {}
        }
    }

    /// 체크 상태 변경 (사용자가 체크 상자를 누름)
    fn ext_notify(&mut self, lp: LPARAM) -> Option<LRESULT> {
        let nm = unsafe { &*(lp.0 as *const NMHDR) };
        if nm.idFrom as u16 != LIST || nm.code != LVN_ITEMCHANGED {
            return None;
        }
        let nl = unsafe { &*(lp.0 as *const NMLISTVIEW) };
        let mask = LVIS_STATEIMAGEMASK.0;
        if (nl.uChanged & LVIF_STATE).0 == 0 || (nl.uNewState & mask) == (nl.uOldState & mask) || nl.iItem < 0 {
            return Some(LRESULT(0));
        }
        let on = (nl.uNewState & mask) >> 12 == 2;
        let Some(&i) = self.ext.as_ref().and_then(|d| d.shown.get(nl.iItem as usize)) else { return Some(LRESULT(0)) };
        let e = exts::catalog()[i].ext;
        if on {
            self.s.exts.enabled.insert(e.to_string());
        } else {
            self.s.exts.enabled.remove(e);
        }
        self.ext_count();
        Some(LRESULT(0))
    }
}

impl Dlg {
    pub fn apply_fonts(&self, f: &super::dpi::Fonts) {
        self.reg.apply(f);
    }
}

//! 정보 창 — 버전, 개발자(github.com/maspo1, 링크), 사용한 오픈소스 목록(검색), 실행 시 쓰는 OS 기능·글꼴.

use super::ctl::{self, FontReg};
use super::dpi::{px, FontKind};
use super::layout::R;
use super::listview::{self, col, Col};
use super::{drain_acts, with_app, Act, App};
use crate::gui::about::{crates, GITHUB, RUNTIME};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const CLASS: PCWSTR = w!("deid_about");
const COLS: [Col; 4] = [col("이름", 190), col("버전", 80), col("라이선스", 190), col("용도", 0)];
const SEARCH: u16 = 500;
const LINK: u16 = 501;
const CLOSE: u16 = 502;
const LOGO: u16 = 503;

pub struct Dlg {
    hwnd: HWND,
    search: HWND,
    list: HWND,
    reg: FontReg,
    /// 로고 비트맵 (STATIC 은 비트맵을 소유하지 않으므로 창이 닫힐 때까지 여기서 보관)
    _logo: Option<super::gdi::Bitmap>,
}

pub fn open(owner: HWND) {
    if let Some(Some(h)) = with_app(owner, |a| a.about.as_ref().map(|d| d.hwnd)) {
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    let dpi = with_app(owner, |a| a.dpi()).unwrap_or(96);
    let (w, h) = (px(680, dpi), px(580, dpi));
    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(owner, &mut rc);
    }
    let title = ctl::wide("deid 정보");
    let created = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            CLASS,
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_CAPTION | WS_SYSMENU,
            rc.left + (rc.right - rc.left - w) / 2,
            rc.top + (rc.bottom - rc.top - h) / 2,
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
                a.about_create(hwnd);
                LRESULT(0)
            }),
            WM_COMMAND => {
                let (cid, code) = ((wp.0 & 0xFFFF) as u16, ((wp.0 >> 16) & 0xFFFF) as u32);
                match (cid, code) {
                    (SEARCH, EN_CHANGE) => with_app(owner, |a| a.about_fill()).map(|_| LRESULT(0)),
                    (LOGO, 0 /* STN_CLICKED */) => {
                        with_app(owner, |a| a.acts.push(Act::Open(GITHUB.into())));
                        Some(LRESULT(0))
                    }
                    (CLOSE, _) | (2 /* IDCANCEL: Esc */, _) => {
                        let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                        Some(LRESULT(0))
                    }
                    _ => None,
                }
            }
            WM_NOTIFY => {
                let nm = &*(lp.0 as *const NMHDR);
                if nm.idFrom as u16 == LINK && (nm.code == NM_CLICK || nm.code == NM_RETURN) {
                    with_app(owner, |a| a.acts.push(Act::Open(GITHUB.into())));
                }
                None
            }
            WM_CLOSE => {
                with_app(owner, |a| a.about = None);
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
    fn about_create(&mut self, hwnd: HWND) {
        let dpi = self.dpi();
        let p = |v| px(v, dpi);
        let mut reg = FontReg::default();
        let r = &mut reg;
        let title = ctl::label(hwnd, &format!("deid   버전 {}", env!("CARGO_PKG_VERSION")), r, FontKind::Bold);
        let desc = ctl::label(hwnd, "파일 속 개인정보·민감정보 비식별화 도구 — 순수 Rust, 단일 실행 파일", r, FontKind::Ui);
        let dev = ctl::label(hwnd, "개발자", r, FontKind::Ui);
        // GitHub 마크 (누르면 링크와 같이 열림). 창 배경색 위에 미리 섞은 불투명 비트맵
        let ls = p(16);
        let bg = unsafe { windows::Win32::Graphics::Gdi::GetSysColor(windows::Win32::Graphics::Gdi::COLOR_BTNFACE) };
        let logo_bmp = super::gdi::Bitmap::from_bgra(ls, ls, &super::logo::github_mark(ls, bg));
        let logo = ctl::child(hwnd, ctl::STATIC, "", 0xE /* SS_BITMAP */ | 0x100 /* SS_NOTIFY */, 0, LOGO, r, FontKind::Ui);
        if let Some(b) = &logo_bmp {
            ctl::send(logo, 0x0172 /* STM_SETIMAGE */, 0 /* IMAGE_BITMAP */, b.handle().0 as isize);
        }
        let link = ctl::child(hwnd, WC_LINK, &format!("<a href=\"{GITHUB}\">github.com/maspo1</a>"), WS_TABSTOP.0, 0, LINK, r, FontKind::Ui);
        let info = ctl::label(hwnd, "화면: Win32 공용 컨트롤 (네이티브)    검사 엔진: 별도 프로세스 (검사할 때만 실행, 끝나면 메모리 반환)", r, FontKind::Ui);
        let n = crates().count();
        let head = ctl::label(hwnd, &format!("사용한 오픈소스 ({n}개)와 실행 시 쓰는 OS 기능·글꼴"), r, FontKind::Bold);
        let search = ctl::edit(hwnd, SEARCH, 0, r, FontKind::Ui);
        ctl::cue(search, "이름·라이선스 검색");
        let list = listview::create(hwnd, 0, false, 0, &COLS, dpi, r, FontKind::Ui);
        let close = ctl::child(hwnd, ctl::BUTTON, "닫기", WS_TABSTOP.0 | 1, 0, CLOSE, r, FontKind::Ui);

        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        let (cw, ch) = (rc.right, rc.bottom);
        let m = p(12);
        let fw = cw - 2 * m;
        for (h, b) in [
            (title, R::new(m, m, fw, p(20))),
            (desc, R::new(m, m + p(24), fw, p(20))),
            (dev, R::new(m, m + p(50), p(60), p(20))),
            (logo, R::new(m + p(64), m + p(52), ls, ls)),
            (link, R::new(m + p(86), m + p(50), fw - p(86), p(20))),
            (info, R::new(m, m + p(74), fw, p(20))),
            (head, R::new(m, m + p(106), fw - p(210), p(20))),
            (search, R::new(cw - m - p(200), m + p(102), p(200), p(24))),
            (list, R::new(m, m + p(132), fw, ch - m - p(132) - p(42))),
            (close, R::new(cw - m - p(100), ch - m - p(30), p(100), p(30))),
        ] {
            ctl::place(h, b);
        }
        listview::fit_columns(list, &COLS, fw, dpi);
        reg.apply(&self.fonts);
        self.about = Some(Dlg { hwnd, search, list, reg, _logo: logo_bmp });
        self.about_fill();
    }

    fn about_fill(&mut self) {
        let Some(d) = &self.about else { return };
        let q = ctl::text(d.search).trim().to_lowercase();
        listview::clear(d.list);
        for [name, ver, lic, _repo, usage] in crates() {
            if q.is_empty() || name.contains(&q) || lic.to_lowercase().contains(&q) {
                listview::push(d.list, &[name, ver, lic, usage]);
            }
        }
        for (a, b, c) in RUNTIME {
            if q.is_empty() || a.to_lowercase().contains(&q) || c.to_lowercase().contains(&q) {
                listview::push(d.list, &[a, "-", c, &format!("실행 시 사용: {b}")]);
            }
        }
    }
}

impl Dlg {
    pub fn apply_fonts(&self, f: &super::dpi::Fonts) {
        self.reg.apply(f);
    }
}

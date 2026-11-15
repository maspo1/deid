//! 컨트롤 생성·조작 공통 함수. 모든 화면 모듈이 이것만 써서 Win32 호출을 한곳에 모은다.

use super::dpi::{FontKind, Fonts};
use super::layout::R;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const BUTTON: PCWSTR = windows::core::w!("BUTTON");
pub const EDIT: PCWSTR = windows::core::w!("EDIT");
pub const STATIC: PCWSTR = windows::core::w!("STATIC");
pub const COMBOBOX: PCWSTR = windows::core::w!("COMBOBOX");

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

pub fn hinst() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|h| HINSTANCE(h.0)).unwrap_or_default() }
}

pub fn send(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    unsafe { SendMessageW(h, msg, WPARAM(w), LPARAM(l)).0 }
}

/// 글꼴을 다시 입혀야 할 컨트롤 목록 (DPI 가 바뀌면 일괄 적용)
#[derive(Default)]
pub struct FontReg(pub Vec<(HWND, FontKind)>);

impl FontReg {
    pub fn apply(&self, fonts: &Fonts) {
        for (h, k) in &self.0 {
            send(*h, WM_SETFONT, fonts.get(*k).0 as usize, 1);
        }
    }
}

/// 자식 컨트롤 생성 (WS_CHILD | WS_VISIBLE 는 항상 붙음)
pub fn child(parent: HWND, class: PCWSTR, text: &str, style: u32, ex: u32, id: u16, reg: &mut FontReg, font: FontKind) -> HWND {
    let t = wide(text);
    let h = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(ex),
            class,
            PCWSTR(t.as_ptr()),
            WINDOW_STYLE(style) | WS_CHILD | WS_VISIBLE,
            0,
            0,
            10,
            10,
            parent,
            HMENU(id as usize as _),
            hinst(),
            None,
        )
    }
    .unwrap_or_default();
    reg.0.push((h, font));
    h
}

pub fn label(parent: HWND, text: &str, reg: &mut FontReg, font: FontKind) -> HWND {
    child(parent, STATIC, text, 0x80 /* SS_NOPREFIX */, 0, 0, reg, font)
}

pub fn button(parent: HWND, text: &str, id: u16, reg: &mut FontReg) -> HWND {
    child(parent, BUTTON, text, WS_TABSTOP.0 | 0 /* BS_PUSHBUTTON */, 0, id, reg, FontKind::Ui)
}

pub fn checkbox(parent: HWND, text: &str, id: u16, reg: &mut FontReg) -> HWND {
    child(parent, BUTTON, text, WS_TABSTOP.0 | 3 /* BS_AUTOCHECKBOX */, 0, id, reg, FontKind::Ui)
}

pub fn edit(parent: HWND, id: u16, extra: u32, reg: &mut FontReg, font: FontKind) -> HWND {
    child(parent, EDIT, "", WS_TABSTOP.0 | 0x80 /* ES_AUTOHSCROLL */ | extra, WS_EX_CLIENTEDGE.0, id, reg, font)
}

pub fn text(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h).max(0) as usize;
        let mut buf = vec![0u16; n + 1];
        let got = GetWindowTextW(h, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..got])
    }
}

/// 내용이 같으면 건드리지 않음 (깜빡임·불필요한 EN_CHANGE 방지)
pub fn set_text(h: HWND, s: &str) {
    if text(h) != s {
        let w = wide(s);
        unsafe {
            let _ = SetWindowTextW(h, PCWSTR(w.as_ptr()));
        }
    }
}

pub fn checked(h: HWND) -> bool {
    send(h, BM_GETCHECK, 0, 0) == 1
}

pub fn set_check(h: HWND, on: bool) {
    send(h, BM_SETCHECK, on as usize, 0);
}

pub fn enable(h: HWND, on: bool) {
    unsafe {
        let _ = EnableWindow(h, on);
    }
}

pub fn show(h: HWND, on: bool) {
    unsafe {
        let _ = ShowWindow(h, if on { SW_SHOWNA } else { SW_HIDE });
    }
}

pub fn place(h: HWND, r: R) {
    unsafe {
        let _ = MoveWindow(h, r.x, r.y, r.w.max(0), r.h.max(0), true);
    }
}

/// 입력칸이 비었을 때 보이는 회색 안내 문구
pub fn cue(h: HWND, s: &str) {
    let w = wide(s);
    send(h, windows::Win32::UI::Controls::EM_SETCUEBANNER, 1, w.as_ptr() as isize);
}

pub fn message(owner: HWND, text: &str, kind: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let (t, c) = (wide(text), wide(crate::gui::sys::APP_TITLE));
    unsafe { MessageBoxW(owner, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), kind) }
}

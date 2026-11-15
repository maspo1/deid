//! 알림 창 — 아이콘(초록 체크: 검사완료·안전 / 파랑 i: 1차 검사 완료 / 주황 !: 경고) + 제목 + 내용 + 버튼.
//! 버튼 하나는 초 단위로 줄어드는 카운트다운을 버튼 글자에 표시하고, 0 이 되면 자동으로 눌린다.
//! 버튼을 누르면 창을 닫고 그 버튼 ID 를 메인 창에 WM_COMMAND 로 보낸다 (닫기·Esc 는 아무 동작 없음).

use super::ctl::{self, FontReg};
use super::dpi::{px, FontKind};
use super::layout::R;
use super::{drain_acts, with_app, App};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const CLASS: PCWSTR = w!("deid_notice");
const TIMER: usize = 1;
const BTN0: u16 = 600;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    Ok,
    Info,
    Warn,
}

pub struct Spec {
    pub icon: Icon,
    pub title: String,
    pub text: String,
    /// (메인 창에 보낼 ID, 글자)
    pub buttons: Vec<(u16, String)>,
    /// (버튼 ID, 초) — 그 버튼에 카운트다운
    pub countdown: Option<(u16, u32)>,
}

pub struct Dlg {
    hwnd: HWND,
    reg: FontReg,
    buttons: Vec<(HWND, u16, String)>,
    countdown: Option<(u16, u32)>,
    _icon: Option<super::gdi::Bitmap>,
}

pub fn open(owner: HWND, spec: Spec) {
    // 이미 떠 있으면 닫고 새로
    if let Some(Some(h)) = with_app(owner, |a| a.notice.as_ref().map(|d| d.hwnd)) {
        unsafe {
            let _ = DestroyWindow(h);
        }
        with_app(owner, |a| a.notice = None);
    }
    let dpi = with_app(owner, |a| {
        a.notice_spec = Some(spec);
        a.dpi()
    })
    .unwrap_or(96);
    let (w, h) = (px(500, dpi), px(250, dpi));
    let mut rc = RECT::default();
    unsafe {
        let _ = GetWindowRect(owner, &mut rc);
    }
    let title = ctl::wide("deid");
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

/// 닫고 (누른 버튼이 있으면) 메인 창에 그 ID 를 보냄
fn finish(hwnd: HWND, owner: HWND, cmd: Option<u16>) {
    unsafe {
        let _ = KillTimer(hwnd, TIMER);
        with_app(owner, |a| a.notice = None);
        let _ = EnableWindow(owner, true);
        let _ = DestroyWindow(hwnd);
        let _ = SetForegroundWindow(owner);
        if let Some(c) = cmd {
            let _ = PostMessageW(owner, WM_COMMAND, WPARAM(c as usize), LPARAM(0));
        }
    }
}

pub extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let owner = GetWindow(hwnd, GW_OWNER).unwrap_or_default();
        let r = match msg {
            WM_CREATE => with_app(owner, |a| {
                a.notice_create(hwnd);
                LRESULT(0)
            }),
            WM_COMMAND => {
                let cid = (wp.0 & 0xFFFF) as u16;
                if cid == 2 {
                    // Esc
                    finish(hwnd, owner, None);
                } else if cid >= BTN0 {
                    let cmd = with_app(owner, |a| a.notice.as_ref().and_then(|d| d.buttons.get((cid - BTN0) as usize).map(|b| b.1))).flatten();
                    finish(hwnd, owner, cmd);
                }
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER => {
                // 카운트다운: 버튼 글자 갱신, 0 이면 자동으로 누름
                let fire = with_app(owner, |a| a.notice.as_mut().and_then(|d| d.tick())).flatten();
                if let Some(c) = fire {
                    finish(hwnd, owner, Some(c));
                }
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                finish(hwnd, owner, None);
                Some(LRESULT(0))
            }
            _ => None,
        };
        drain_acts(owner);
        r.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
    }
}

impl Dlg {
    fn label(text: &str, secs: Option<u32>) -> String {
        secs.map_or(text.to_string(), |s| format!("{text} ({s}s)"))
    }

    /// 1초마다. 0 이 되면 누를 버튼 ID
    fn tick(&mut self) -> Option<u16> {
        let (id, left) = self.countdown.as_mut()?;
        *left = left.saturating_sub(1);
        let (id, left) = (*id, *left);
        if let Some((h, _, t)) = self.buttons.iter().find(|b| b.1 == id) {
            ctl::set_text(*h, &Dlg::label(t, Some(left)));
        }
        (left == 0).then_some(id)
    }

    pub fn apply_fonts(&self, f: &super::dpi::Fonts) {
        self.reg.apply(f);
    }
}

impl App {
    fn notice_create(&mut self, hwnd: HWND) {
        let Some(spec) = self.notice_spec.take() else { return };
        let dpi = self.dpi();
        let p = |v| px(v, dpi);
        let mut reg = FontReg::default();
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        let (cw, ch) = (rc.right, rc.bottom);
        let m = p(16);
        // 아이콘
        let is = p(40);
        let bg = unsafe { windows::Win32::Graphics::Gdi::GetSysColor(windows::Win32::Graphics::Gdi::COLOR_BTNFACE) };
        let icon = super::gdi::Bitmap::from_bgra(is, is, &icon_pixels(is, spec.icon, bg));
        let ih = ctl::child(hwnd, ctl::STATIC, "", 0xE /* SS_BITMAP */, 0, 0, &mut reg, FontKind::Ui);
        if let Some(b) = &icon {
            ctl::send(ih, 0x0172 /* STM_SETIMAGE */, 0, b.handle().0 as isize);
        }
        ctl::place(ih, R::new(m, m, is, is));
        let tx = m + is + p(14);
        let title = ctl::label(hwnd, &spec.title, &mut reg, FontKind::Bold);
        ctl::place(title, R::new(tx, m, cw - tx - m, p(22)));
        let text = ctl::label(hwnd, &spec.text, &mut reg, FontKind::Ui);
        ctl::place(text, R::new(tx, m + p(28), cw - tx - m, ch - m - p(28) - p(52)));
        // 버튼 (오른쪽 정렬, 첫 버튼이 기본)
        let mut buttons = Vec::new();
        let bh = p(30);
        let widths: Vec<i32> = spec.buttons.iter().map(|(_, t)| p((t.chars().count() as i32 * 13 + 44).clamp(90, 260))).collect();
        let mut x = cw - m - widths.iter().sum::<i32>() - p(8) * (widths.len() as i32 - 1).max(0);
        for (i, ((cmd, t), bw)) in spec.buttons.iter().zip(&widths).enumerate() {
            let secs = spec.countdown.filter(|c| c.0 == *cmd).map(|c| c.1);
            let style = WS_TABSTOP.0 | if i == 0 { 1 /* BS_DEFPUSHBUTTON */ } else { 0 };
            let h = ctl::child(hwnd, ctl::BUTTON, &Dlg::label(t, secs), style, 0, BTN0 + i as u16, &mut reg, FontKind::Ui);
            ctl::place(h, R::new(x, ch - m - bh, *bw, bh));
            x += bw + p(8);
            buttons.push((h, *cmd, t.clone()));
        }
        reg.apply(&self.fonts);
        if let Some((h, ..)) = buttons.first() {
            unsafe {
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(*h);
            }
        }
        if spec.countdown.is_some() {
            unsafe {
                SetTimer(hwnd, TIMER, 1000, None);
            }
        }
        self.notice = Some(Dlg { hwnd, reg, buttons, countdown: spec.countdown, _icon: icon });
    }
}

/// 원 + 흰 표시(체크 / 느낌표)를 4×4 표본으로 부드럽게, 창 배경색 `bg`(0x00BBGGRR) 위에 미리 섞은 불투명 BGRA
pub fn icon_pixels(size: i32, kind: Icon, bg: u32) -> Vec<u32> {
    let n = size.max(8) as usize;
    let f = n as f64;
    let (cr, cg, cb) = match kind {
        Icon::Ok => (0x1Eu32, 0x9Eu32, 0x4Au32),   // 초록
        Icon::Info => (0x1Eu32, 0x6Fu32, 0xD9u32), // 파랑
        Icon::Warn => (0xE0u32, 0x8Au32, 0x00u32), // 주황
    };
    let seg = |px_: f64, py: f64, (ax, ay): (f64, f64), (bx, by): (f64, f64)| {
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((px_ - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((px_ - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
    };
    let stroke = f * 0.085;
    let in_mark = |x: f64, y: f64| match kind {
        Icon::Ok => seg(x, y, (0.27 * f, 0.52 * f), (0.44 * f, 0.68 * f)).min(seg(x, y, (0.44 * f, 0.68 * f), (0.74 * f, 0.34 * f))) <= stroke,
        Icon::Warn => seg(x, y, (0.5 * f, 0.24 * f), (0.5 * f, 0.56 * f)) <= stroke || ((x - 0.5 * f).powi(2) + (y - 0.73 * f).powi(2)).sqrt() <= stroke * 1.2,
        Icon::Info => seg(x, y, (0.5 * f, 0.44 * f), (0.5 * f, 0.76 * f)) <= stroke || ((x - 0.5 * f).powi(2) + (y - 0.27 * f).powi(2)).sqrt() <= stroke * 1.2,
    };
    let (br, bgc, bb) = (bg & 0xFF, (bg >> 8) & 0xFF, (bg >> 16) & 0xFF);
    let mut out = vec![0u32; n * n];
    for y in 0..n {
        for x in 0..n {
            let (mut circle, mut mark) = (0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let (fx, fy) = (x as f64 + (sx as f64 + 0.5) / 4.0, y as f64 + (sy as f64 + 0.5) / 4.0);
                    if ((fx - f / 2.0).powi(2) + (fy - f / 2.0).powi(2)).sqrt() <= f / 2.0 - 0.5 {
                        circle += 1;
                        mark += in_mark(fx, fy) as u32;
                    }
                }
            }
            // 바탕 → 원 색 → 흰 표시 순서로 덮음
            let mix = |c: u32, b: u32| (c * (circle - mark) + 255 * mark + b * (16 - circle)) / 16;
            out[y * n + x] = 0xFF00_0000 | mix(cr, br) << 16 | mix(cg, bgc) << 8 | mix(cb, bb);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_circle_and_white_mark() {
        let n = 40;
        let px = icon_pixels(n, Icon::Ok, 0x00F0_F0F0);
        let at = |x: i32, y: i32| px[(y * n + x) as usize] & 0xFF_FFFF;
        assert_eq!(at(0, 0), 0xF0_F0F0, "모서리 = 창 배경");
        assert_eq!(at(20, 6), 0x1E_9E4A, "원 안 = 초록");
        assert_eq!(at(17, 27), 0xFF_FFFF, "체크 꺾이는 곳 = 흰색");
    }
}

//! 왼쪽 옵션 패널 — 세로 스크롤 되는 자식 창 안에 검출 항목·엔진·OCR·파일·사용자 정의 정규식.
//! 값의 원본은 `Settings` 하나이고, 컨트롤이 바뀔 때마다 `read` 로 옮긴다.

use super::ctl::{self, child, send, FontReg};
use super::dpi::{px, FontKind};
use super::layout::{R, PANEL_W};
use crate::common::InfoType;
use crate::gui::settings::Settings;
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Controls::{TRACKBAR_CLASSW, TBM_SETPOS, TBM_SETRANGEMAX, TBM_SETRANGEMIN};

/// 슬라이더 위치 읽기 (= WM_USER, windows 크레이트에 상수가 없음)
const TBM_GETPOS: u32 = 0x0400;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const CLASS: windows::core::PCWSTR = w!("deid_panel");

/// 패널 컨트롤 ID (200번대)
pub mod oid {
    pub const TYPE0: u16 = 180; // ~192 (InfoType::BUILTIN 13개)
    pub const ALL: u16 = 212;
    pub const NONE: u16 = 213;
    pub const ML: u16 = 214;
    pub const OCR: u16 = 216;
    pub const SCALES: u16 = 217;
    pub const MP: u16 = 218;
    pub const ENHANCE: u16 = 219;
    pub const RENAME: u16 = 220;
    pub const SCAN_EXE: u16 = 221;
    pub const MAX_MB: u16 = 222;
    pub const MIN_RUN: u16 = 223;
    pub const EXTS: u16 = 224;
    pub const PATTERNS: u16 = 225;
    pub const LOAD: u16 = 226;
    pub const TRY: u16 = 227;
    pub const DEFAULTS: u16 = 228;
    pub const THR: u16 = 230;
    pub const BLUR: u16 = 231;
}

pub struct Panel {
    pub hwnd: HWND,
    /// (컨트롤, 96 DPI 기준 위치 — 스크롤 전)
    items: Vec<(HWND, R)>,
    /// 사용 중지할 입력 컨트롤 (검사 중)
    inputs: Vec<HWND>,
    height: i32,
    scroll: i32,
    rect: R,
    pub types: Vec<HWND>,
    pub ml: HWND,
    pub thr: HWND,
    thr_val: HWND,
    pub ocr: HWND,
    scales: HWND,
    mp: HWND,
    enhance: HWND,
    blur: HWND,
    blur_val: HWND,
    rename: HWND,
    scan_exe: HWND,
    max_mb: HWND,
    min_run: HWND,
    exts: HWND,
    pub patterns: HWND,
    pub errors: HWND,
}

const CW: i32 = PANEL_W - 17 - 12; // 내용 폭 (스크롤바·여백 제외)
const X: i32 = 6;

struct Builder<'a> {
    parent: HWND,
    reg: &'a mut FontReg,
    items: Vec<(HWND, R)>,
    inputs: Vec<HWND>,
    y: i32,
}

impl Builder<'_> {
    fn put(&mut self, h: HWND, x: i32, w: i32, hgt: i32) -> HWND {
        self.items.push((h, R::new(X + x, self.y, w, hgt)));
        h
    }
    fn input(&mut self, h: HWND, x: i32, w: i32, hgt: i32) -> HWND {
        self.inputs.push(h);
        self.put(h, x, w, hgt)
    }
    fn section(&mut self, title: &str) {
        self.y += 6;
        let l = ctl::label(self.parent, title, self.reg, FontKind::Bold);
        self.put(l, 0, CW, 18);
        self.y += 20;
        // 홈 선: SS_ETCHEDHORZ 는 2px 선만 그리고 나머지(배율 125% 이상에서 생기는 줄)를 지우지 않아
        // 스크롤 잔상이 남는다 → 영역 전체를 칠하는 빈 STATIC + 오목 테두리
        let sep = child(self.parent, ctl::STATIC, "", 0x1000 /* SS_SUNKEN */, 0, 0, self.reg, FontKind::Ui);
        self.put(sep, 0, CW, 2);
        self.y += 6;
    }
    fn label(&mut self, text: &str, x: i32, w: i32) -> HWND {
        let l = ctl::label(self.parent, text, self.reg, FontKind::Ui);
        self.items.push((l, R::new(X + x, self.y + 4, w, 18)));
        l
    }
    fn check(&mut self, text: &str, id: u16, x: i32, w: i32) -> HWND {
        let h = ctl::checkbox(self.parent, text, id, self.reg);
        self.input(h, x, w, 20)
    }
    fn button(&mut self, text: &str, id: u16, x: i32, w: i32) -> HWND {
        let h = ctl::button(self.parent, text, id, self.reg);
        self.input(h, x, w, 24)
    }
    fn edit(&mut self, id: u16, x: i32, w: i32, numeric: bool) -> HWND {
        let h = ctl::edit(self.parent, id, if numeric { 0x2000 /* ES_NUMBER */ } else { 0 }, self.reg, FontKind::Ui);
        self.input(h, x, w, 22)
    }
    fn slider(&mut self, id: u16, lo: i32, hi: i32) -> (HWND, HWND) {
        let t = child(self.parent, TRACKBAR_CLASSW, "", WS_TABSTOP.0 | 0x10 /* TBS_NOTICKS */, 0, id, self.reg, FontKind::Ui);
        send(t, TBM_SETRANGEMIN, 0, lo as isize);
        send(t, TBM_SETRANGEMAX, 1, hi as isize);
        self.input(t, 74, 150, 24);
        let v = ctl::label(self.parent, "", self.reg, FontKind::Ui);
        self.items.push((v, R::new(X + 228, self.y + 4, 40, 18)));
        (t, v)
    }
}

impl Panel {
    pub fn create(parent: HWND, reg: &mut FontReg) -> Panel {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                CLASS,
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_VSCROLL | WS_CLIPCHILDREN | WS_TABSTOP,
                0,
                0,
                10,
                10,
                parent,
                None,
                ctl::hinst(),
                None,
            )
        }
        .unwrap_or_default();
        let mut b = Builder { parent: hwnd, reg, items: vec![], inputs: vec![], y: 0 };

        b.section("검출 항목");
        b.label("개인정보", 0, 120);
        b.label("민감정보", 140, 120);
        b.y += 20;
        let mut types = Vec::new();
        for (i, t) in InfoType::BUILTIN.iter().enumerate() {
            let (col, row) = if i < 6 { (0, i) } else { (1, i - 6) };
            let save = b.y;
            b.y += row as i32 * 22;
            types.push(b.check(t.label(), oid::TYPE0 + i as u16, col * 140, 136));
            b.y = save;
        }
        b.y += 7 * 22 + 2;
        b.button("모두 선택", oid::ALL, 0, 80);
        b.button("모두 해제", oid::NONE, 86, 80);
        b.y += 28;

        b.section("탐지 엔진");
        let ml = b.check("XGBoost 맥락 탐지 (기호가 바뀐 번호 추론)", oid::ML, 0, CW);
        b.y += 24;
        b.label("채택 확률", 0, 70);
        let (thr, thr_val) = b.slider(oid::THR, 50, 99);
        b.y += 30;

        b.section("그림 (OCR)");
        let ocr = b.check("그림 속 글자 검사 — 찾은 정보는 흐리게", oid::OCR, 0, CW);
        b.y += 24;
        b.label("확대 배율", 0, 70);
        let scales = b.edit(oid::SCALES, 74, 80, false);
        b.y += 26;
        b.label("화소 상한", 0, 70);
        let mp = b.edit(oid::MP, 74, 80, false);
        b.label("백만 화소", 160, 80);
        b.y += 26;
        let enhance = b.check("글자 또렷하게 (흑백 변환 후 인식)", oid::ENHANCE, 0, CW);
        b.y += 24;
        b.label("흐림 강도", 0, 70);
        let (blur, blur_val) = b.slider(oid::BLUR, 1, 100);
        b.y += 30;

        b.section("파일");
        let rename = b.check("파일·폴더 이름의 정보도 비식별화", oid::RENAME, 0, CW);
        b.y += 22;
        let scan_exe = b.check("실행 파일(exe, dll)도 검사", oid::SCAN_EXE, 0, CW);
        b.y += 26;
        b.label("최대 파일 크기", 0, 120);
        let max_mb = b.edit(oid::MAX_MB, 124, 70, true);
        b.label("MB", 200, 30);
        b.y += 26;
        b.label("바이너리 문자열 최소 길이", 0, 160);
        let min_run = b.edit(oid::MIN_RUN, 164, 50, true);
        b.y += 30;
        let exts = b.button("", oid::EXTS, 0, CW);
        b.y += 32;

        b.section("사용자 정의 정규식");
        let help = ctl::label(hwnd, "한 줄에 하나씩  라벨=정규식\n(그룹1이 있으면 그룹1만 치환, #은 주석)", b.reg, FontKind::Ui);
        b.put(help, 0, CW, 34);
        b.y += 36;
        let patterns = ctl::edit(hwnd, oid::PATTERNS, 0x4 | 0x1000 | 0x40 /* MULTILINE|WANTRETURN|AUTOVSCROLL */ | WS_VSCROLL.0, b.reg, FontKind::Mono);
        b.input(patterns, 0, CW, 96);
        b.y += 100;
        b.button("파일 불러오기...", oid::LOAD, 0, 120);
        b.button("미리보기에서 시험", oid::TRY, 126, 130);
        b.y += 28;
        let errors = ctl::label(hwnd, "", b.reg, FontKind::Ui);
        b.put(errors, 0, CW, 36);
        b.y += 38;
        b.button("모든 설정 기본값으로", oid::DEFAULTS, 0, 160);
        b.y += 34;

        let (items, inputs, height) = (b.items, b.inputs, b.y);
        Panel {
            hwnd,
            items,
            inputs,
            height,
            scroll: 0,
            rect: R::default(),
            types,
            ml,
            thr,
            thr_val,
            ocr,
            scales,
            mp,
            enhance,
            blur,
            blur_val,
            rename,
            scan_exe,
            max_mb,
            min_run,
            exts,
            patterns,
            errors,
        }
    }

    /// 패널 크기·DPI·스크롤 위치에 맞게 컨트롤 배치
    pub fn layout(&mut self, r: R, dpi: u32) {
        self.rect = r;
        ctl::place(self.hwnd, r);
        let total = px(self.height, dpi);
        self.scroll = self.scroll.clamp(0, (total - r.h).max(0));
        let si = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: total - 1,
            nPage: r.h.max(0) as u32,
            nPos: self.scroll,
            ..Default::default()
        };
        unsafe {
            windows::Win32::UI::Controls::SetScrollInfo(self.hwnd, SB_VERT, &si, true);
        }
        // 컨트롤마다 MoveWindow 로 옮기면 옛 화면 비트가 복사되어 잔상이 남는다 →
        // 비트 복사·개별 다시 그리기 없이 모두 옮긴 뒤 패널 전체를 한 번 다시 그림
        unsafe {
            let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOREDRAW;
            for (h, it) in &self.items {
                let _ = SetWindowPos(*h, None, px(it.x, dpi), px(it.y, dpi) - self.scroll, px(it.w, dpi), px(it.h, dpi), flags);
            }
            use windows::Win32::Graphics::Gdi::{RedrawWindow, RDW_ALLCHILDREN, RDW_ERASE, RDW_INVALIDATE, RDW_UPDATENOW};
            let _ = RedrawWindow(self.hwnd, None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW);
        }
    }

    /// WM_VSCROLL / WM_MOUSEWHEEL
    pub fn on_scroll(&mut self, msg: u32, wp: usize, dpi: u32) {
        let line = px(24, dpi);
        let mut si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_ALL, ..Default::default() };
        unsafe {
            let _ = GetScrollInfo(self.hwnd, SB_VERT, &mut si);
        }
        let page = si.nPage as i32;
        self.scroll = if msg == WM_MOUSEWHEEL {
            let delta = (wp >> 16) as u16 as i16 as i32;
            self.scroll - delta * line * 3 / 120
        } else {
            match (wp & 0xFFFF) as u32 {
                x if x == SB_LINEUP.0 as u32 => self.scroll - line,
                x if x == SB_LINEDOWN.0 as u32 => self.scroll + line,
                x if x == SB_PAGEUP.0 as u32 => self.scroll - page,
                x if x == SB_PAGEDOWN.0 as u32 => self.scroll + page,
                x if x == SB_THUMBTRACK.0 as u32 || x == SB_THUMBPOSITION.0 as u32 => si.nTrackPos,
                x if x == SB_TOP.0 as u32 => 0,
                x if x == SB_BOTTOM.0 as u32 => i32::MAX,
                _ => self.scroll,
            }
        };
        self.layout(self.rect, dpi);
    }

    /// Settings → 컨트롤
    pub fn write(&self, s: &Settings) {
        for (h, on) in self.types.iter().zip(s.enabled) {
            ctl::set_check(*h, on);
        }
        ctl::set_check(self.ml, s.ml);
        send(self.thr, TBM_SETPOS, 1, (s.threshold * 100.0).round() as isize);
        ctl::set_check(self.ocr, s.ocr);
        ctl::set_text(self.scales, &s.ocr_scales);
        ctl::set_check(self.enhance, s.ocr_enhance);
        ctl::set_text(self.mp, &format!("{}", s.ocr_mp));
        send(self.blur, TBM_SETPOS, 1, (s.blur * 10.0).round() as isize);
        ctl::set_check(self.rename, s.rename);
        ctl::set_check(self.scan_exe, s.scan_exe);
        ctl::set_text(self.max_mb, &s.max_mb.to_string());
        ctl::set_text(self.min_run, &s.min_run.to_string());
        ctl::set_text(self.patterns, &s.patterns.replace("\r\n", "\n").replace('\n', "\r\n"));
        self.refresh_labels(s);
    }

    /// 컨트롤 → Settings (숫자가 잘못되면 이전 값 유지; 범위 검증은 Settings::options 가 담당)
    pub fn read(&self, s: &mut Settings) {
        for (i, h) in self.types.iter().enumerate() {
            s.enabled[i] = ctl::checked(*h);
        }
        s.ml = ctl::checked(self.ml);
        s.threshold = send(self.thr, TBM_GETPOS, 0, 0) as f32 / 100.0;
        s.ocr = ctl::checked(self.ocr);
        s.ocr_scales = ctl::text(self.scales);
        s.ocr_enhance = ctl::checked(self.enhance);
        if let Ok(v) = ctl::text(self.mp).trim().parse() {
            s.ocr_mp = v;
        }
        s.blur = send(self.blur, TBM_GETPOS, 0, 0) as f32 / 10.0;
        s.rename = ctl::checked(self.rename);
        s.scan_exe = ctl::checked(self.scan_exe);
        if let Ok(v) = ctl::text(self.max_mb).trim().parse() {
            s.max_mb = v;
        }
        if let Ok(v) = ctl::text(self.min_run).trim().parse() {
            s.min_run = v;
        }
        s.patterns = ctl::text(self.patterns).replace("\r\n", "\n");
        self.refresh_labels(s);
    }

    /// 값 표시·사용 여부·정규식 오류처럼 설정에 딸린 글자 갱신
    pub fn refresh_labels(&self, s: &Settings) {
        ctl::set_text(self.thr_val, &format!("{:.2}", s.threshold));
        ctl::set_text(self.blur_val, &format!("{:.1}", s.blur));
        ctl::set_text(self.exts, &format!("검사할 확장자 선택 ({}/{})...", s.exts.enabled.len(), crate::exts::catalog().len()));
        ctl::set_text(self.errors, &s.pattern_errors().join("\n"));
    }

    /// 검사 중에는 입력 중지. OCR 이 꺼지면 OCR 세부 항목도 중지.
    pub fn set_enabled(&self, on: bool, s: &Settings) {
        for h in &self.inputs {
            ctl::enable(*h, on);
        }
        for h in [self.scales, self.mp, self.enhance, self.blur] {
            ctl::enable(h, on && s.ocr);
        }
    }
}

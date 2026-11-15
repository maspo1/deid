//! 모니터별 DPI(Per-Monitor V2) — 배율 계산과 DPI 별 글꼴 묶음.
//! 매니페스트에 PerMonitorV2 가 있고, 혹시 매니페스트가 빠진 경우를 대비해 실행 시에도 한 번 더 요청한다.

use super::gdi::Font;
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

pub fn enable_per_monitor_v2() {
    // 이미 매니페스트로 설정됐으면 실패(ACCESS_DENIED) — 무시해도 된다
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// 96 DPI 기준 값 → 현재 DPI 화소 (반올림)
pub fn px(v: i32, dpi: u32) -> i32 {
    ((v as i64 * dpi as i64 + 48) / 96) as i32
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontKind {
    /// 맑은 고딕 9pt
    Ui,
    /// 맑은 고딕 9pt 굵게 (섹션 제목)
    Bold,
    /// Consolas 9pt (로그, 정규식, 값) — 한글은 OS 글꼴 연결로 맑은 고딕
    Mono,
}

/// 한 DPI 에 대한 글꼴 묶음. DPI 가 바뀌면 통째로 새로 만들고 이전 것은 Drop 으로 해제.
pub struct Fonts {
    pub dpi: u32,
    ui: Font,
    bold: Font,
    mono: Font,
}

impl Fonts {
    pub fn new(dpi: u32) -> Fonts {
        Fonts {
            dpi,
            ui: Font::new("Malgun Gothic", 9, false, dpi),
            bold: Font::new("Malgun Gothic", 9, true, dpi),
            mono: Font::new("Consolas", 10, false, dpi),
        }
    }

    pub fn get(&self, k: FontKind) -> HFONT {
        match k {
            FontKind::Ui => self.ui.handle(),
            FontKind::Bold => self.bold.handle(),
            FontKind::Mono => self.mono.handle(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn px_rounds() {
        assert_eq!(super::px(100, 96), 100);
        assert_eq!(super::px(100, 120), 125);
        assert_eq!(super::px(100, 144), 150);
        assert_eq!(super::px(7, 144), 11); // 10.5 → 11
    }
}

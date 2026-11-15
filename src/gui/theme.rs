//! 글꼴·색·간격 — 한글 글꼴은 OS 기본 글꼴(Windows 맑은 고딕, macOS 애플 SD 산돌고딕 Neo)을 메모리 매핑해 쓴다 (exe 에 포함하지 않음).

use egui::{self, Color32, FontData, FontDefinitions, FontFamily, Visuals};
use std::sync::Arc;

/// 강조색 (진행 막대, 시작 버튼)
pub const ACCENT: Color32 = Color32::from_rgb(0x2B, 0x6C, 0xD6);

/// 한글 글꼴 후보: (경로, ttc 안의 번호) — OS 기본 글꼴만 사용
fn korean_fonts() -> Vec<(std::path::PathBuf, u32)> {
    #[cfg(windows)]
    {
        let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
        [("malgun.ttf", 0), ("gulim.ttc", 0), ("batang.ttc", 0)].iter().map(|(f, i)| (dir.join(f), *i)).collect()
    }
    #[cfg(target_os = "macos")]
    {
        [("/System/Library/Fonts/AppleSDGothicNeo.ttc", 0), ("/System/Library/Fonts/Supplemental/AppleGothic.ttf", 0)]
            .iter()
            .map(|(f, i)| (f.into(), *i))
            .collect()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        [("/usr/share/fonts/truetype/nanum/NanumGothic.ttf", 0), ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 1)]
            .iter()
            .map(|(f, i)| (f.into(), *i))
            .collect()
    }
}

/// 글꼴 파일을 메모리 매핑 — 힙에 복사하지 않으므로 개인 메모리를 쓰지 않는다 (OS 파일 캐시와 공유)
fn map_font(path: &std::path::Path) -> Option<&'static [u8]> {
    let f = std::fs::File::open(path).ok()?;
    // 안전: 시스템 글꼴 파일은 실행 중 바뀌지 않는다. 매핑은 프로세스 종료까지 유지(의도적 누수).
    let m = unsafe { memmap2::Mmap::map(&f) }.ok()?;
    Some(&Box::leak(Box::new(m))[..])
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    if let Some((bytes, index)) = korean_fonts().iter().find_map(|(p, i)| map_font(p).map(|b| (b, *i))) {
        let mut fd = FontData::from_static(bytes);
        fd.index = index;
        fonts.font_data.insert("korean".into(), Arc::new(fd));
        // 본문: 한글 글꼴 우선 / 고정폭: 영문은 고정폭, 한글만 대체
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "korean".into());
        fonts.families.entry(FontFamily::Monospace).or_default().push("korean".into());
    }
    ctx.set_fonts(fonts);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 24.0;
        // 고전 목록처럼: 글자를 눌러도 행이 선택되도록 라벨 텍스트 선택 끔 (복사는 우클릭 메뉴)
        s.interaction.selectable_labels = false;
        for (style, size) in [
            (egui::TextStyle::Body, 14.0),
            (egui::TextStyle::Button, 14.0),
            (egui::TextStyle::Monospace, 13.0),
            (egui::TextStyle::Small, 12.0),
            (egui::TextStyle::Heading, 19.0),
        ] {
            if let Some(f) = s.text_styles.get_mut(&style) {
                f.size = size;
            }
        }
    });
    for (theme, mut v) in [(egui::Theme::Light, Visuals::light()), (egui::Theme::Dark, Visuals::dark())] {
        v.selection.bg_fill = ACCENT;
        v.selection.stroke.color = Color32::WHITE;
        v.hyperlink_color = ACCENT;
        ctx.set_visuals_of(theme, v);
    }
}

/// 섹션 제목
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).strong().color(ui.visuals().strong_text_color()));
    ui.separator();
}

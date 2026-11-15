//! 확장자 선택 창 — 동영상 플레이어의 "지원 확장자 선택" 같은 분류별 체크 목록.

use crate::exts::{self, ExtFilter};

#[derive(Default)]
pub struct ExtWindow {
    pub open: bool,
    /// None = 전체 분류
    category: Option<&'static str>,
    search: String,
}

impl ExtWindow {
    pub fn show(&mut self, ctx: &egui::Context, f: &mut ExtFilter, enabled: bool) {
        let mut open = self.open;
        egui::Window::new("검사할 확장자 선택")
            .open(&mut open)
            .default_size([560.0, 480.0])
            .min_width(420.0)
            .collapsible(false)
            .show(ctx, |ui| ui.add_enabled_ui(enabled, |ui| self.body(ui, f)));
        self.open = open;
    }

    fn body(&mut self, ui: &mut egui::Ui, f: &mut ExtFilter) {
        ui.label(
            egui::RichText::new("체크한 확장자만 검사합니다. 실제 형식은 항상 파일 내용(매직바이트)으로 판별하며, 체크하지 않은 확장자는 원본 그대로 복사됩니다.")
                .small(),
        );
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("ext_cat")
                .selected_text(self.category.unwrap_or("전체 분류"))
                .width(190.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.category, None, "전체 분류");
                    for c in exts::categories() {
                        ui.selectable_value(&mut self.category, Some(c), c);
                    }
                });
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("확장자·설명 검색").desired_width(150.0));
        });

        let q = self.search.trim().to_lowercase();
        let shown: Vec<&exts::ExtInfo> = exts::catalog()
            .iter()
            .filter(|e| self.category.map_or(true, |c| e.category == c))
            .filter(|e| q.is_empty() || e.ext.contains(q.as_str()) || e.desc.to_lowercase().contains(q.as_str()))
            .collect();

        ui.horizontal(|ui| {
            if ui.button("보이는 항목 선택").clicked() {
                shown.iter().for_each(|e| {
                    f.enabled.insert(e.ext.to_string());
                });
            }
            if ui.button("보이는 항목 해제").clicked() {
                shown.iter().for_each(|e| {
                    f.enabled.remove(e.ext);
                });
            }
            if ui.button("기본값").clicked() {
                f.enabled = ExtFilter::default().enabled;
            }
        });

        let footer = 70.0;
        egui::ScrollArea::vertical().max_height((ui.available_height() - footer).max(120.0)).auto_shrink([false, false]).show(ui, |ui| {
            let cols = ((ui.available_width() / 128.0).floor() as usize).max(2);
            // 분류별 격자
            for cat in exts::categories() {
                let items: Vec<&&exts::ExtInfo> = shown.iter().filter(|e| e.category == cat).collect();
                if items.is_empty() {
                    continue;
                }
                let n_on = items.iter().filter(|x| f.enabled.contains(x.ext)).count();
                egui::CollapsingHeader::new(format!("{cat}  ({n_on}/{})", items.len()))
                    .id_salt(cat)
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new(("ext_grid", cat)).num_columns(cols).spacing([12.0, 2.0]).show(ui, |ui| {
                            for (i, e) in items.iter().enumerate() {
                                let mut on = f.enabled.contains(e.ext);
                                if ui.checkbox(&mut on, format!(".{}", e.ext)).on_hover_text(e.desc).changed() {
                                    if on {
                                        f.enabled.insert(e.ext.to_string());
                                    } else {
                                        f.enabled.remove(e.ext);
                                    }
                                }
                                if (i + 1) % cols == 0 {
                                    ui.end_row();
                                }
                            }
                        });
                    });
            }
        });

        ui.separator();
        ui.checkbox(&mut f.scan_unknown, "목록에 없는 확장자와 확장자 없는 파일도 검사 (매직바이트로 판별)");
        ui.checkbox(&mut f.check_disguised, "확장자를 위장한 파일은 체크하지 않았어도 검사 (예: 이름만 .log 인 Word 문서)");
        ui.label(format!("선택 {} / {}", f.enabled.len(), exts::catalog().len()));
    }
}

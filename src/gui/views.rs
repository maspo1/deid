//! 가운데 영역 탭: 로그, 검출 결과 표(사람 검토용), 미리보기(정규식·탐지 시험). 파일 분석은 `forensic.rs`.

use super::scan::{Scan, Src};
use super::settings::Settings;
use super::sys;
use crate::common::mask_text;
use crate::rules::{Detector, Lang};
use egui_extras::{Column, TableBuilder};

#[derive(PartialEq, Clone, Copy, Default)]
pub enum Tab {
    #[default]
    Log,
    Results,
    Forensic,
    Preview,
}

// ───────────────────────────── 로그 ─────────────────────────────

pub fn log(ui: &mut egui::Ui, lines: &[String]) {
    let h = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
    egui::ScrollArea::both().stick_to_bottom(true).auto_shrink([false, false]).show_rows(ui, h, lines.len(), |ui, range| {
        for l in &lines[range] {
            ui.add(egui::Label::new(egui::RichText::new(l).monospace()).extend());
        }
    });
}

// ───────────────────────────── 검출 결과 ─────────────────────────────

#[derive(Default)]
pub struct ResultFilter {
    pub review_only: bool,
    pub kind: Option<String>,
    pub search: String,
    /// (필터 키, 검사 버전) → 보이는 (위치, 번호)
    cache: Option<((bool, Option<String>, String, u64), Vec<(Src, usize)>)>,
}

impl ResultFilter {
    fn visible(&mut self, scan: &Scan) -> Vec<(Src, usize)> {
        let key = (self.review_only, self.kind.clone(), self.search.trim().to_lowercase(), scan.version);
        if self.cache.as_ref().map_or(true, |(k, _)| *k != key) {
            let srcs = (0..scan.names.len()).map(Src::Name).chain((0..scan.entries.len()).map(Src::File));
            let mut idx = Vec::new();
            for s in srcs {
                let rel = scan.rel(scan.path(s)).to_lowercase();
                for (k, r) in scan.records(s).iter().enumerate() {
                    let ok = (!key.0 || r.review)
                        && key.1.as_ref().map_or(true, |t| r.base() == t.as_str())
                        && (key.2.is_empty() || r.original.to_lowercase().contains(&key.2) || rel.contains(&key.2));
                    if ok {
                        idx.push((s, k));
                    }
                }
            }
            self.cache = Some((key, idx));
        }
        self.cache.as_ref().map(|c| c.1.clone()).unwrap_or_default()
    }
}

/// 타입별 건수 (표 위 요약)
fn counts(scan: &Scan) -> Vec<(String, usize)> {
    let mut m: std::collections::BTreeMap<&str, usize> = Default::default();
    for r in scan.all_records() {
        *m.entry(r.base()).or_default() += 1;
    }
    let mut v: Vec<_> = m.into_iter().map(|(k, n)| (k.to_string(), n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v
}

pub fn results(ui: &mut egui::Ui, scan: Option<&Scan>, f: &mut ResultFilter) {
    let Some(scan) = scan.filter(|s| s.all_records().next().is_some()) else {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.weak("검사를 마치면 검출된 정보가 여기에 표시됩니다. 보고서(CSV)와 같은 내용입니다."));
        return;
    };
    let total = scan.all_records().count();
    let review = scan.all_records().filter(|r| r.review).count();
    let kinds = counts(scan);
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(format!("총 {total}건")).strong());
        if review > 0 {
            ui.label(egui::RichText::new(format!("검토필요 {review}건")).color(ui.visuals().warn_fg_color).strong());
        }
        for (k, n) in &kinds {
            ui.weak(format!("· {k} {n}"));
        }
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut f.review_only, "검토필요만");
        egui::ComboBox::from_id_salt("kind_filter")
            .selected_text(f.kind.clone().unwrap_or_else(|| "모든 타입".into()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.kind, None, "모든 타입");
                for (k, _) in &kinds {
                    ui.selectable_value(&mut f.kind, Some(k.clone()), k);
                }
            });
        ui.add(egui::TextEdit::singleline(&mut f.search).hint_text("검출 정보·경로 검색").desired_width(200.0));
    });
    if review > 0 {
        ui.label(
            egui::RichText::new("※ 애매한 항목도 일단 치환했습니다. 노란 행을 확인하고, 정보가 아니면 원본에서 해당 부분을 되살리세요.")
                .small()
                .color(ui.visuals().warn_fg_color),
        );
    }
    ui.add_space(2.0);

    let vis = f.visible(scan);
    let warn = ui.visuals().warn_fg_color;
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt("results_table")
            .striped(true)
            .resizable(true)
            .auto_shrink([false, false])
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(170.0).at_least(60.0).clip(true))
            .column(Column::initial(170.0).at_least(60.0).clip(true))
            .column(Column::initial(130.0).at_least(60.0).clip(true))
            .column(Column::remainder().at_least(120.0).clip(true))
            .header(row_h, |mut h| {
                for t in ["검출된정보", "치환정보", "정보타입", "파일경로"] {
                    h.col(|ui| {
                        ui.strong(t);
                    });
                }
            })
            .body(|body| {
                body.rows(row_h, vis.len(), |mut row| {
                    let (s, k) = vis[row.index()];
                    let r = &scan.records(s)[k];
                    let tint = |t: &str| {
                        let rt = egui::RichText::new(t);
                        if r.review {
                            rt.color(warn)
                        } else {
                            rt
                        }
                    };
                    row.col(|ui| {
                        ui.add(egui::Label::new(tint(&r.original).monospace()).truncate()).on_hover_text(&r.original);
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&r.masked).monospace()).truncate());
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(tint(&r.label)).truncate());
                    });
                    row.col(|ui| {
                        let resp = ui
                            .add(egui::Label::new(scan.rel(scan.path(s))).truncate().sense(egui::Sense::click()))
                            .on_hover_text("우클릭: 파일 위치 열기");
                        resp.context_menu(|ui| {
                            if ui.button("원본 파일 위치 열기").clicked() {
                                sys::reveal(scan.path(s));
                                ui.close_menu();
                            }
                            if let Some(o) = scan.out_path(s) {
                                if ui.button("결과 파일 위치 열기").clicked() {
                                    sys::reveal(o);
                                    ui.close_menu();
                                }
                            }
                            if ui.button("검출된 정보 복사").clicked() {
                                ui.ctx().copy_text(r.original.clone());
                                ui.close_menu();
                            }
                        });
                    });
                });
            });
    });
}

// ───────────────────────────── 미리보기 ─────────────────────────────

/// 현재 설정으로 문장을 바로 검사해 본다 (사용자 정의 정규식 시험용)
#[derive(Default)]
pub struct Preview {
    pub text: String,
    /// (입력, 설정 지문) → 결과
    cache: Option<(String, String, Result<(String, Vec<(String, String, String)>), String>)>,
}

fn fingerprint(s: &Settings) -> String {
    format!("{:?}{}{}{}", s.enabled, s.ml, s.threshold, s.patterns)
}

pub fn preview(ui: &mut egui::Ui, p: &mut Preview, s: &Settings) {
    ui.label("현재 설정(검출 항목·맥락 탐지·사용자 정의 정규식)으로 문장을 바로 검사합니다. 파일은 바뀌지 않습니다.");
    ui.add(
        egui::TextEdit::multiline(&mut p.text)
            .hint_text("예) 담당자 010-1234-5678, server 192-168-0-1, password=hunter2")
            .font(egui::TextStyle::Monospace)
            .desired_rows(6)
            .desired_width(f32::INFINITY),
    );
    let fp = fingerprint(s);
    if p.cache.as_ref().map_or(true, |(t, f, _)| *t != p.text || *f != fp) {
        let r = s.options().map(|o| {
            let det = Detector::new(o);
            let (masked, recs) = mask_text(&p.text, &det.scan(&p.text, Lang::None));
            (masked, recs.iter().map(|r| (r.original.clone(), r.masked.clone(), r.label())).collect())
        });
        p.cache = Some((p.text.clone(), fp, r));
    }
    let Some((_, _, r)) = &p.cache else { return };
    match r {
        Err(e) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        Ok((masked, recs)) => {
            ui.add_space(4.0);
            ui.strong("치환 결과");
            let mut m = masked.clone();
            ui.add(egui::TextEdit::multiline(&mut m).font(egui::TextStyle::Monospace).desired_rows(4).desired_width(f32::INFINITY).interactive(false));
            ui.strong(format!("검출 {}건", recs.len()));
            egui::Grid::new("preview_grid").striped(true).num_columns(3).show(ui, |ui| {
                for (o, m, l) in recs {
                    ui.monospace(o);
                    ui.monospace(m);
                    ui.label(l);
                    ui.end_row();
                }
            });
        }
    }
}

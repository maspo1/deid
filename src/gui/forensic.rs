//! 파일 분석 (포렌식 보기) — Process Explorer 의 트리 표 + NetworkMiner 의 상세 창 구조.
//!
//! * 위: 폴더 트리 표. 검사한 모든 파일의 형식(매직바이트), 크기, 검출·검토 건수, 상태를 색으로 구분.
//!   파일의 [+] 를 펼치면 NetworkMiner 처럼 정보 타입별 묶음 → 값으로 내려간다
//! * 아래: 선택한 파일(폴더)의 상세 — 검출 정보 목록 / 파일 정보

use super::scan::{Scan, Src};
use super::sys;
use crate::ipc::{Rec, Status};
use egui::{Color32, RichText};
use egui_extras::{Column, TableBuilder};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Default, Clone, Copy)]
struct Agg {
    files: usize,
    recs: usize,
    review: usize,
    masked: usize,
    size: u64,
}

impl Agg {
    fn add(&mut self, o: &Agg) {
        self.files += o.files;
        self.recs += o.recs;
        self.review += o.review;
        self.masked += o.masked;
        self.size += o.size;
    }
}

#[derive(Clone)]
enum NodeKind {
    /// 상대 경로 (접기 상태 키)
    Dir(String),
    File(usize),
    /// 펼친 파일 안의 정보 타입 묶음 (접기 상태 키, 파일 번호)
    Type(String, usize),
    /// 묶음 안의 값 1건 (파일 번호, 검출 위치, 번호)
    Item(usize, Src, usize),
}

struct Node {
    name: String,
    depth: usize,
    kind: NodeKind,
    agg: Agg,
    /// 이름에서 나온 검출 (names 의 번호)
    name_hit: Option<usize>,
}

#[derive(Clone, PartialEq, Debug)]
enum Sel {
    File(usize),
    Dir(String),
}

#[derive(PartialEq, Clone, Copy, Default)]
enum DetailTab {
    #[default]
    Records,
    Props,
}

#[derive(Default)]
pub struct Forensic {
    nodes: Vec<Node>,
    built: Option<(u64, bool, String, u64)>,
    /// 접힌 폴더·타입 묶음 (기본 펼침)
    collapsed: HashSet<String>,
    /// 펼친 파일 (기본 접힘)
    expanded: HashSet<usize>,
    expand_ver: u64,
    selected: Option<Sel>,
    only_found: bool,
    search: String,
    tab: DetailTab,
    /// 파일별 [+] 상자 위치 (테스트에서 실제 클릭용)
    #[cfg(test)]
    boxes: HashMap<usize, egui::Rect>,
}

/// 타입 묶음의 접기 키 (폴더 경로와 겹치지 않도록 제어 문자로 시작)
fn type_key(file: usize, label: &str) -> String {
    format!("\x01{file}:{label}")
}

#[derive(Default)]
struct DirTree {
    dirs: BTreeMap<String, DirTree>,
    files: Vec<(String, usize)>,
}

fn review_count(r: &[Rec]) -> usize {
    r.iter().filter(|x| x.review).count()
}

impl Forensic {
    pub fn reset(&mut self) {
        *self = Forensic { only_found: self.only_found, ..Default::default() };
    }

    fn rebuild(&mut self, scan: &Scan) {
        let key = (scan.version, self.only_found, self.search.trim().to_lowercase(), self.expand_ver);
        if self.built.as_ref() == Some(&key) {
            return;
        }
        let q = key.2.clone();
        let mut root = DirTree::default();
        for (i, e) in scan.entries.iter().enumerate() {
            if self.only_found && e.records.is_empty() {
                continue;
            }
            let rel = scan.rel(&e.path);
            if !q.is_empty() && !rel.to_lowercase().contains(&q) {
                continue;
            }
            let parts: Vec<&str> = rel.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
            let Some((file, dirs)) = parts.split_last() else { continue };
            let mut node = &mut root;
            for d in dirs {
                node = node.dirs.entry(d.to_string()).or_default();
            }
            node.files.push((file.to_string(), i));
        }
        let name_of: HashMap<String, usize> = scan.names.iter().enumerate().map(|(i, n)| (scan.rel(&n.path), i)).collect();
        let mut nodes = Vec::new();
        flatten(&root, "", 0, scan, &name_of, &self.expanded, &mut nodes);
        self.nodes = nodes;
        self.built = Some(key);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, scan: Option<&Scan>) {
        let Some(scan) = scan else {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.weak("검사를 시작하면 검사한 모든 파일이 폴더 구조 그대로 여기에 표시됩니다."));
            return;
        };
        // 얇은 떠 있는 스크롤바는 마우스를 올려야 보인다 → 이 보기에서는 항상 보이는 고전 스크롤바
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        self.rebuild(scan);
        self.toolbar(ui, scan);

        egui::TopBottomPanel::bottom("forensic_detail").resizable(true).default_height(220.0).min_height(90.0).show_inside(ui, |ui| {
            self.details(ui, scan);
        });
        egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| self.tree(ui, scan));
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, scan: &Scan) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("모두 펼치기").clicked() {
                self.collapsed.clear();
                self.expanded = scan.entries.iter().enumerate().filter(|(_, e)| !e.records.is_empty()).map(|(i, _)| i).collect();
                self.expand_ver += 1;
            }
            if ui.button("모두 접기").clicked() {
                self.collapsed = self.nodes.iter().filter_map(|n| if let NodeKind::Dir(p) = &n.kind { Some(p.clone()) } else { None }).collect();
                self.expanded.clear();
                self.expand_ver += 1;
            }
            ui.checkbox(&mut self.only_found, "정보가 있는 파일만");
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("파일 이름·경로 검색").desired_width(170.0));
            ui.separator();
            let dark = ui.visuals().dark_mode;
            for s in [Status::Masked, Status::Found, Status::Skipped, Status::Error] {
                legend(ui, tint(s, false, dark), s.label());
            }
            legend(ui, tint(Status::Masked, true, dark), "검토필요");
            ui.separator();
            let total: u64 = scan.entries.iter().map(|e| e.size).sum();
            ui.weak(format!("파일 {}개 · {}", scan.entries.len(), human(total)));
        });
        ui.add_space(2.0);
    }

    fn visible(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut hide_below: Option<usize> = None;
        for (i, n) in self.nodes.iter().enumerate() {
            if let Some(d) = hide_below {
                if n.depth > d {
                    continue;
                }
                hide_below = None;
            }
            out.push(i);
            if let NodeKind::Dir(p) | NodeKind::Type(p, _) = &n.kind {
                if self.collapsed.contains(p) {
                    hide_below = Some(n.depth);
                }
            }
        }
        out
    }

    fn tree(&mut self, ui: &mut egui::Ui, scan: &Scan) {
        let vis = self.visible();
        let dark = ui.visuals().dark_mode;
        let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
        let mut toggle: Option<String> = None;
        let mut toggle_file: Option<usize> = None;
        let mut select: Option<Sel> = None;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt("forensic_tree")
                .striped(false)
                .resizable(true)
                .sense(egui::Sense::click())
                .auto_shrink([false, false])
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(260.0).at_least(120.0).clip(true))
                .column(Column::initial(110.0).at_least(50.0).clip(true))
                .column(Column::initial(80.0).at_least(50.0).clip(true))
                .column(Column::initial(48.0).at_least(36.0))
                .column(Column::initial(48.0).at_least(36.0))
                .column(Column::initial(52.0).at_least(40.0))
                .column(Column::remainder().at_least(100.0).clip(true))
                .header(row_h + 2.0, |mut h| {
                    for t in ["이름", "형식 (매직바이트)", "크기", "검출", "검토", "상태", "비고"] {
                        h.col(|ui| {
                            ui.strong(t);
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h, vis.len(), |mut row| {
                        let n = &self.nodes[vis[row.index()]];
                        // 타입 묶음·값 행을 누르면 그 파일을 선택
                        let sel = match &n.kind {
                            NodeKind::Dir(p) => Sel::Dir(p.clone()),
                            NodeKind::File(i) | NodeKind::Type(_, i) | NodeKind::Item(i, ..) => Sel::File(*i),
                        };
                        let own_row = matches!(n.kind, NodeKind::Dir(_) | NodeKind::File(_));
                        row.set_selected(own_row && self.selected.as_ref() == Some(&sel));
                        let (bg, status_txt, kind_txt, note) = match &n.kind {
                            NodeKind::Dir(_) => {
                                let bg = (n.agg.recs > 0).then(|| dir_tint(dark));
                                (bg, String::new(), "폴더".to_string(), format!("파일 {}개", n.agg.files))
                            }
                            NodeKind::File(i) => {
                                let e = &scan.entries[*i];
                                let review = review_count(&e.records) > 0;
                                let bg = (e.status != Status::Clean).then(|| tint(e.status, review, dark));
                                (bg, e.status.label().to_string(), e.kind.clone(), e.notes.join(" / "))
                            }
                            NodeKind::Type(..) => (None, String::new(), String::new(), String::new()),
                            NodeKind::Item(_, s, k) => {
                                let r = &scan.records(*s)[*k];
                                (None, if r.review { "검토필요".into() } else { String::new() }, String::new(), r.label.clone())
                            }
                        };
                        let paint = |ui: &mut egui::Ui| {
                            if let Some(c) = bg {
                                let r = ui.max_rect().expand2(egui::vec2(ui.spacing().item_spacing.x * 0.5 + 1.0, 1.0));
                                ui.painter().rect_filled(r, 0.0, c);
                            }
                        };
                        row.col(|ui| {
                            paint(ui);
                            ui.add_space(n.depth as f32 * 14.0);
                            match &n.kind {
                                NodeKind::Dir(p) | NodeKind::Type(p, _) => {
                                    if expander(ui, !self.collapsed.contains(p)).clicked() {
                                        toggle = Some(p.clone());
                                    }
                                    ui.label(RichText::new(&n.name).strong());
                                }
                                NodeKind::File(i) => {
                                    if n.agg.recs == 0 {
                                        ui.add_space(11.0 + ui.spacing().item_spacing.x);
                                    } else {
                                        let b = expander(ui, self.expanded.contains(i));
                                        #[cfg(test)]
                                        let _ = self.boxes.insert(*i, b.rect);
                                        if b.clicked() {
                                            toggle_file = Some(*i);
                                            select = Some(sel.clone());
                                        }
                                    }
                                    ui.add(egui::Label::new(&n.name).truncate());
                                }
                                NodeKind::Item(_, s, k) => {
                                    ui.add_space(11.0 + ui.spacing().item_spacing.x);
                                    let t = RichText::new(&n.name).monospace();
                                    let t = if scan.records(*s)[*k].review { t.color(warn(dark)) } else { t };
                                    ui.add(egui::Label::new(t).truncate());
                                }
                            }
                            if n.name_hit.is_some() {
                                ui.label(RichText::new("이름").small().color(warn(dark)));
                            }
                        });
                        row.col(|ui| {
                            paint(ui);
                            ui.add(egui::Label::new(RichText::new(kind_txt).small()).truncate());
                        });
                        row.col(|ui| {
                            paint(ui);
                            if own_row {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(human(n.agg.size)));
                            }
                        });
                        row.col(|ui| {
                            paint(ui);
                            if n.agg.recs > 0 && !matches!(n.kind, NodeKind::Item(..)) {
                                ui.label(RichText::new(n.agg.recs.to_string()).strong());
                            }
                        });
                        row.col(|ui| {
                            paint(ui);
                            if n.agg.review > 0 && !matches!(n.kind, NodeKind::Item(..)) {
                                ui.label(RichText::new(n.agg.review.to_string()).color(warn(dark)).strong());
                            }
                        });
                        row.col(|ui| {
                            paint(ui);
                            ui.label(status_txt);
                        });
                        row.col(|ui| {
                            paint(ui);
                            ui.add(egui::Label::new(RichText::new(note).small()).truncate());
                        });
                        let resp = row.response();
                        if resp.clicked() {
                            select = Some(sel.clone());
                        }
                        if resp.double_clicked() {
                            match &n.kind {
                                NodeKind::Dir(p) | NodeKind::Type(p, _) => toggle = Some(p.clone()),
                                NodeKind::File(i) => sys::reveal(&scan.entries[*i].out),
                                NodeKind::Item(..) => {}
                            }
                        }
                    });
                });
        });
        let changed = toggle.is_some() || toggle_file.is_some() || (select.is_some() && select != self.selected);
        if let Some(p) = toggle {
            if !self.collapsed.remove(&p) {
                self.collapsed.insert(p);
            }
        }
        if let Some(i) = toggle_file {
            if !self.expanded.remove(&i) {
                self.expanded.insert(i);
            }
            self.expand_ver += 1;
        }
        if select.is_some() {
            self.selected = select;
        }
        // 상세 창은 트리보다 먼저 그려지므로, 바뀐 선택·펼침이 이 프레임에 바로 보이도록 다시 그린다
        if changed {
            ui.ctx().request_discard("파일 분석 선택·펼침 변경");
        }
    }

    fn details(&mut self, ui: &mut egui::Ui, scan: &Scan) {
        ui.add_space(4.0);
        let Some(sel) = self.selected.clone() else {
            ui.weak("파일이나 폴더를 고르면 찾은 정보와 파일 정보가 여기에 표시됩니다. (두 번 클릭: 결과 파일 위치 열기)");
            return;
        };
        // 대상의 검출 목록 (파일: 내용 + 이름, 폴더: 이름 + 하위 전체)
        let (title, recs): (String, Vec<(Src, usize)>) = match &sel {
            Sel::File(i) => {
                let e = &scan.entries[*i];
                let mut v: Vec<(Src, usize)> = (0..e.records.len()).map(|k| (Src::File(*i), k)).collect();
                if let Some(n) = scan.names.iter().position(|n| n.path == e.path) {
                    v.extend((0..scan.names[n].records.len()).map(|k| (Src::Name(n), k)));
                }
                (scan.rel(&e.path), v)
            }
            Sel::Dir(p) => {
                let prefix = format!("{p}{}", std::path::MAIN_SEPARATOR);
                let under = |path: &std::path::Path| {
                    let r = scan.rel(path);
                    r == *p || r.starts_with(&prefix)
                };
                let mut v = Vec::new();
                for (i, e) in scan.entries.iter().enumerate() {
                    if under(&e.path) {
                        v.extend((0..e.records.len()).map(|k| (Src::File(i), k)));
                    }
                }
                for (i, n) in scan.names.iter().enumerate() {
                    if under(&n.path) {
                        v.extend((0..n.records.len()).map(|k| (Src::Name(i), k)));
                    }
                }
                (format!("{p}{}", std::path::MAIN_SEPARATOR), v)
            }
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(&title).strong());
            ui.separator();
            ui.selectable_value(&mut self.tab, DetailTab::Records, format!("검출 정보 ({})", recs.len()));
            ui.selectable_value(&mut self.tab, DetailTab::Props, "파일 정보");
        });
        ui.separator();
        match self.tab {
            DetailTab::Records => records_table(ui, scan, &recs, matches!(sel, Sel::Dir(_))),
            DetailTab::Props => match &sel {
                Sel::File(i) => props(ui, scan, *i),
                Sel::Dir(p) => {
                    let agg = self.nodes.iter().find(|n| matches!(&n.kind, NodeKind::Dir(x) if x == p)).map(|n| n.agg).unwrap_or_default();
                    egui::Grid::new("dir_props").num_columns(2).striped(true).show(ui, |ui| {
                        for (k, v) in [
                            ("원본 경로", scan.root.join(p).display().to_string()),
                            ("파일 수", agg.files.to_string()),
                            ("전체 크기", human(agg.size)),
                            ("치환한 파일", agg.masked.to_string()),
                            ("검출", format!("{}건 (검토필요 {}건)", agg.recs, agg.review)),
                        ] {
                            ui.label(RichText::new(k).weak());
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                }
            },
        }
    }
}

fn flatten(
    t: &DirTree,
    prefix: &str,
    depth: usize,
    scan: &Scan,
    names: &HashMap<String, usize>,
    expanded: &HashSet<usize>,
    out: &mut Vec<Node>,
) -> Agg {
    let mut total = Agg::default();
    for (name, sub) in &t.dirs {
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}{}{name}", std::path::MAIN_SEPARATOR) };
        let at = out.len();
        let name_hit = names.get(&rel).copied();
        out.push(Node { name: name.clone(), depth, kind: NodeKind::Dir(rel.clone()), agg: Agg::default(), name_hit });
        let mut agg = flatten(sub, &rel, depth + 1, scan, names, expanded, out);
        if let Some(n) = name_hit {
            agg.recs += scan.names[n].records.len();
            agg.review += review_count(&scan.names[n].records);
        }
        out[at].agg = agg;
        total.add(&agg);
    }
    for (name, i) in &t.files {
        let e = &scan.entries[*i];
        let rel = scan.rel(&e.path);
        let name_hit = names.get(&rel).copied();
        let mut agg = Agg {
            files: 1,
            recs: e.records.len(),
            review: review_count(&e.records),
            masked: (e.status == Status::Masked) as usize,
            size: e.size,
        };
        if let Some(n) = name_hit {
            agg.recs += scan.names[n].records.len();
            agg.review += review_count(&scan.names[n].records);
        }
        out.push(Node { name: name.clone(), depth, kind: NodeKind::File(*i), agg, name_hit });
        if expanded.contains(i) {
            push_types(*i, name_hit, depth + 1, scan, out);
        }
        total.add(&agg);
    }
    total
}

/// 펼친 파일 아래 행: 정보 타입별 묶음(내용 + 파일명 검출) → 값
fn push_types(file: usize, name_hit: Option<usize>, depth: usize, scan: &Scan, out: &mut Vec<Node>) {
    let mut srcs = vec![Src::File(file)];
    srcs.extend(name_hit.map(Src::Name));
    let mut by: BTreeMap<&str, Vec<(Src, usize)>> = BTreeMap::new();
    for s in srcs {
        for (k, r) in scan.records(s).iter().enumerate() {
            by.entry(r.base()).or_default().push((s, k));
        }
    }
    for (label, items) in by {
        let review = items.iter().filter(|(s, k)| scan.records(*s)[*k].review).count();
        let agg = Agg { recs: items.len(), review, ..Default::default() };
        out.push(Node { name: format!("{label} ({})", items.len()), depth, kind: NodeKind::Type(type_key(file, label), file), agg, name_hit: None });
        for (s, k) in items {
            let r = &scan.records(s)[k];
            let agg = Agg { recs: 1, review: r.review as usize, ..Default::default() };
            out.push(Node { name: format!("{} → {}", r.original, r.masked), depth: depth + 1, kind: NodeKind::Item(file, s, k), agg, name_hit: None });
        }
    }
}

/// 고전 트리의 [+] / [-] 상자
fn expander(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(11.0, 11.0), egui::Sense::click());
    let stroke = egui::Stroke::new(1.0_f32, ui.visuals().text_color());
    let p = ui.painter();
    let r = r.shrink(0.5);
    p.rect_filled(r, 0.0, ui.visuals().extreme_bg_color);
    p.rect_stroke(r, 0.0, stroke, egui::StrokeKind::Inside);
    let c = r.center();
    p.line_segment([egui::pos2(r.left() + 2.5, c.y), egui::pos2(r.right() - 2.5, c.y)], stroke);
    if !open {
        p.line_segment([egui::pos2(c.x, r.top() + 2.5), egui::pos2(c.x, r.bottom() - 2.5)], stroke);
    }
    resp
}

fn records_table(ui: &mut egui::Ui, scan: &Scan, recs: &[(Src, usize)], show_path: bool) {
    if recs.is_empty() {
        ui.weak("찾은 정보가 없습니다.");
        return;
    }
    // 타입별 건수 요약 (NetworkMiner 의 Host Details 처럼)
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for (s, k) in recs {
        *by.entry(scan.records(*s)[*k].base()).or_default() += 1;
    }
    ui.horizontal_wrapped(|ui| {
        for (k, n) in &by {
            ui.label(RichText::new(format!("{k} {n}")).small().strong());
            ui.weak("·");
        }
    });
    let warn_c = warn(ui.visuals().dark_mode);
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
    let mut t = TableBuilder::new(ui)
        .id_salt("forensic_recs")
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::initial(200.0).at_least(60.0).clip(true))
        .column(Column::initial(200.0).at_least(60.0).clip(true))
        .column(Column::initial(150.0).at_least(60.0).clip(true));
    if show_path {
        t = t.column(Column::remainder().at_least(80.0).clip(true));
    }
    t.header(row_h, |mut h| {
        let mut heads = vec!["검출된정보", "치환정보", "정보타입"];
        if show_path {
            heads.push("파일경로");
        }
        for x in heads {
            h.col(|ui| {
                ui.strong(x);
            });
        }
    })
    .body(|body| {
        body.rows(row_h, recs.len(), |mut row| {
            let (s, k) = recs[row.index()];
            let r = &scan.records(s)[k];
            let c = |t: &str| if r.review { RichText::new(t).color(warn_c) } else { RichText::new(t) };
            row.col(|ui| {
                ui.add(egui::Label::new(c(&r.original).monospace()).truncate()).on_hover_text(&r.original).context_menu(|ui| {
                    if ui.button("복사").clicked() {
                        ui.ctx().copy_text(r.original.clone());
                        ui.close_menu();
                    }
                });
            });
            row.col(|ui| {
                ui.add(egui::Label::new(RichText::new(&r.masked).monospace()).truncate());
            });
            row.col(|ui| {
                ui.label(c(&r.label));
            });
            if show_path {
                row.col(|ui| {
                    ui.add(egui::Label::new(scan.rel(scan.path(s))).truncate());
                });
            }
        });
    });
}

fn props(ui: &mut egui::Ui, scan: &Scan, i: usize) {
    let e = &scan.entries[i];
    let ext = crate::exts::ext_of(&e.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let catalog = ext.as_deref().and_then(crate::exts::lookup).map(|x| format!("{} — {}", x.category, x.desc)).unwrap_or_else(|| "목록에 없음".into());
    egui::Grid::new("file_props").num_columns(2).striped(true).show(ui, |ui| {
        let rows = [
            ("원본 경로", e.path.display().to_string()),
            ("결과 경로", e.out.display().to_string()),
            ("실제 형식 (매직바이트)", e.kind.clone()),
            ("확장자", ext.clone().map(|x| format!(".{x}")).unwrap_or_else(|| "없음".into())),
            ("확장자 분류", catalog),
            ("크기", format!("{} ({} bytes)", human(e.size), e.size)),
            ("상태", e.status.label().to_string()),
            ("검출", format!("{}건 (검토필요 {}건)", e.records.len(), review_count(&e.records))),
        ];
        for (k, v) in rows {
            ui.label(RichText::new(k).weak());
            ui.add(egui::Label::new(v).wrap());
            ui.end_row();
        }
        for n in &e.notes {
            ui.label(RichText::new("알림").weak());
            ui.add(egui::Label::new(n).wrap());
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        if ui.button("원본 위치 열기").clicked() {
            sys::reveal(&e.path);
        }
        if ui.button("결과 위치 열기").clicked() {
            sys::reveal(&e.out);
        }
    });
}

// ───────────────────────────── 색 ─────────────────────────────

fn tint(s: Status, review: bool, dark: bool) -> Color32 {
    let (l, d) = match (s, review) {
        (_, true) => ((255, 236, 160), (92, 78, 24)),
        (Status::Masked, _) => ((255, 208, 208), (96, 40, 44)),
        (Status::Found, _) => ((226, 210, 255), (64, 48, 100)),
        (Status::Skipped, _) => ((222, 222, 222), (58, 58, 58)),
        (Status::Error, _) => ((150, 150, 150), (30, 30, 30)),
        (Status::Clean, _) => ((255, 255, 255), (27, 27, 27)),
    };
    let (r, g, b) = if dark { d } else { l };
    Color32::from_rgb(r, g, b)
}

fn dir_tint(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(34, 52, 82)
    } else {
        Color32::from_rgb(208, 228, 255)
    }
}

fn warn(dark: bool) -> Color32 {
    if dark {
        Color32::from_rgb(255, 200, 80)
    } else {
        Color32::from_rgb(176, 96, 0)
    }
}

fn legend(ui: &mut egui::Ui, c: Color32, label: &str) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 2.0, c);
    ui.painter().rect_stroke(r, 2.0, ui.visuals().widgets.noninteractive.bg_stroke, egui::StrokeKind::Inside);
    ui.label(RichText::new(label).small());
}

pub fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

#[cfg(test)]
mod tests {
    //! 화면 없이 egui 프레임을 직접 돌려 클릭을 재현한다 (실제 바탕화면을 건드리지 않음).
    //! `DEID_SNAPSHOT=<폴더>` 를 주면 CPU 렌더러로 그린 화면을 PNG 로 저장한다.
    use super::*;
    use crate::ipc::FileEntry;
    use egui::{pos2, vec2, Event, PointerButton, RawInput, Rect};

    const W: f32 = 900.0;
    const H: f32 = 600.0;

    fn scan(n: usize) -> Scan {
        let root = std::path::PathBuf::from("C:/in");
        let mut s = Scan::new(root.clone());
        for i in 0..n {
            let records = if i % 3 == 0 {
                vec![
                    Rec { original: format!("010-1234-{i:04}"), masked: "***-****-****".into(), label: "핸드폰번호".into(), review: false },
                    Rec { original: "10.0.0.1".into(), masked: "**.*.*.*".into(), label: "IP주소(검토필요)".into(), review: true },
                ]
            } else {
                vec![]
            };
            s.entries.push(FileEntry {
                path: root.join(format!("d{}/f{i:03}.txt", i % 2)),
                out: root.join("o.txt"),
                kind: "TEXT(UTF-8)".into(),
                size: 100,
                status: if records.is_empty() { Status::Clean } else { Status::Masked },
                notes: vec![],
                records,
            });
        }
        s.version = 1;
        s
    }

    fn visit(s: &egui::Shape, f: &mut dyn FnMut(&str)) {
        match s {
            egui::Shape::Text(t) => f(&t.galley.job.text),
            egui::Shape::Vec(v) => v.iter().for_each(|x| visit(x, f)),
            _ => {}
        }
    }

    struct Rig {
        ctx: egui::Context,
        f: Forensic,
        s: Scan,
        raster: super::super::raster::Raster,
        frame: usize,
        /// 마지막 프레임에 그려진 글자들
        texts: Vec<String>,
    }

    impl Rig {
        fn new(n: usize) -> Rig {
            let ctx = egui::Context::default();
            super::super::theme::install(&ctx);
            let mut r = Rig { ctx, f: Forensic::default(), s: scan(n), raster: Default::default(), frame: 0, texts: vec![] };
            r.run(vec![]);
            r.run(vec![]);
            r
        }

        fn run(&mut self, events: Vec<Event>) {
            // 프레임마다 0.15초: 누름→뗌은 짧은 클릭, 클릭 사이(3프레임)는 두 번 클릭 간격(0.3초)보다 길게
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(W, H))),
                time: Some(self.frame as f64 * 0.15),
                events,
                ..Default::default()
            };
            let (f, s) = (&mut self.f, &self.s);
            let out = self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| f.ui(ui, Some(s)));
            });
            self.texts.clear();
            for cs in &out.shapes {
                visit(&cs.shape, &mut |t| self.texts.push(t.to_string()));
            }
            self.raster.set_textures(&out.textures_delta);
            if let Ok(dir) = std::env::var("DEID_SNAPSHOT") {
                let (w, h) = (W as usize, H as usize);
                let mut buf = vec![0u32; w * h];
                let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);
                self.raster.render(&mut buf, w, h, &prims, out.pixels_per_point, egui::Color32::WHITE);
                let rgb: Vec<u8> = buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
                let path = std::path::Path::new(&dir).join(format!("forensic_{}.png", self.frame));
                image::RgbImage::from_raw(W as u32, H as u32, rgb).unwrap().save(path).unwrap();
            }
            self.raster.free_textures(&out.textures_delta);
            self.frame += 1;
        }

        /// 실제 마우스처럼 이동 → 누름 → 뗌 (프레임마다 하나)
        fn click(&mut self, p: egui::Pos2) {
            let m = egui::Modifiers::default();
            self.run(vec![Event::PointerMoved(p)]);
            self.run(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: m }]);
            self.run(vec![Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: m }]);
        }

        fn shows(&self, needle: &str) -> bool {
            self.texts.iter().any(|t| t.contains(needle))
        }
    }

    /// 행을 클릭한 바로 그 프레임에 상세 창이 그 파일 정보를 보여야 한다 (다음 입력을 기다리지 않음)
    #[test]
    fn click_shows_details_in_same_frame() {
        let mut r = Rig::new(30);
        // 첫 줄 = 폴더 d0, 둘째 줄 = f000.txt (검출 2건)
        r.click(pos2(200.0, 132.0));
        assert_eq!(r.f.selected, Some(Sel::File(0)), "f000.txt 가 선택돼야 함");
        assert!(r.shows("검출 정보 (2)"), "클릭한 프레임에 상세 창이 갱신되지 않음: {:?}", r.texts.iter().filter(|t| t.contains("고르면")).collect::<Vec<_>>());
    }

    /// [+] 를 펼치면 NetworkMiner 처럼 정보 타입별 묶음과 값이 나온다
    #[test]
    fn expanded_file_lists_types_and_values() {
        let mut r = Rig::new(30);
        assert!(!r.shows("핸드폰번호 (1)"), "기본은 접힘");
        r.f.expanded.insert(0);
        r.f.expand_ver += 1;
        r.run(vec![]);
        assert!(r.shows("핸드폰번호 (1)"), "타입 묶음 없음");
        assert!(r.shows("IP주소 (1)"), "검토필요 표시를 뺀 타입으로 묶여야 함");
        assert!(r.shows("010-1234-0000"), "값 없음");
        // 타입 묶음을 접으면 값이 사라진다
        let key = type_key(0, "핸드폰번호");
        r.f.collapsed.insert(key);
        r.run(vec![]);
        assert!(r.shows("핸드폰번호 (1)") && !r.shows("010-1234-0000"));
    }

    /// 파일 옆 [+] 상자를 실제로 클릭하면 펼쳐진다
    #[test]
    fn clicking_plus_box_expands() {
        let mut r = Rig::new(30);
        let b = r.f.boxes.get(&0).copied().expect("f000.txt 에 [+] 상자가 그려져야 함");
        r.click(b.center());
        assert!(r.f.expanded.contains(&0), "[+] 클릭으로 펼쳐지지 않음");
        assert!(r.shows("핸드폰번호 (1)"));
        assert_eq!(r.f.selected, Some(Sel::File(0)), "펼친 파일이 선택돼야 함");
    }
}

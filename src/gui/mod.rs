//! GUI — 순수 Rust 네이티브 GUI (egui). GPU 를 쓰지 않는 CPU 렌더러로 그려 메모리를 적게 쓴다.
//!
//! 프로세스 구조 (크롬·엣지·웹뷰와 같은 방식)
//! * 화면 프로세스: 창, 설정, 결과 목록만 보유
//! * 엔진 프로세스: 검사할 때만 `deid --engine` 으로 띄우고 끝나면 종료 (`crate::ipc`)
//!
//! 화면 구성 (Process Explorer / NetworkMiner 형식)
//! * 메뉴: 파일 · 보기 · 도움말(정보)
//! * 위    : 검사 폴더 / 출력 폴더
//! * 왼쪽  : 검출 항목, 탐지 엔진, 그림(OCR), 파일, 사용자 정의 정규식
//! * 가운데: 탭 — 로그 / 검출 결과 / 파일 분석(포렌식 트리 + 상세) / 미리보기
//! * 아래  : 진행 막대, 시작·중지, 보고서·출력 폴더 열기
//!
//! 모듈
//! * `host` 창·이벤트 루프, `raster` CPU 렌더러, `theme` 글꼴·색
//! * `settings` 입력값 ↔ Options, `worker` 엔진 프로세스, `scan` 결과 모음
//! * `views` 로그·결과 표·미리보기, `forensic` 파일 분석, `ext_window` 확장자 선택, `about` 정보
//! * `sys` OS 기능(대화상자·탐색기), `win` Windows 전용 구현

pub(crate) mod about;
mod ext_window;
mod forensic;
mod host;
mod raster;
pub(crate) mod scan;
pub(crate) mod settings;
pub(crate) mod sys;
mod theme;
mod views;
#[cfg(windows)]
mod win;
mod worker;

#[cfg(windows)]
pub use win::use_cli_console;

use crate::common::InfoType;
use egui::RichText;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use scan::Scan;
use settings::Settings;
use std::path::PathBuf;
use std::process::ExitCode;
use sys::Owner;
use views::Tab;
use worker::Handle;

pub struct App {
    s: Settings,
    ext_win: ext_window::ExtWindow,
    about: about::About,
    job: Option<Handle>,
    stopping: bool,
    scan: Option<Scan>,
    tab: Tab,
    filter: views::ResultFilter,
    forensic: forensic::Forensic,
    preview: views::Preview,
    error: Option<String>,
    confirm_close: bool,
    /// 호스트가 이 값을 보고 창을 닫는다
    pub quit: bool,
    owner: Owner,
}

pub fn run(prefill: Option<PathBuf>) -> ExitCode {
    let mut s = Settings::default();
    if let Some(p) = prefill.filter(|p| p.is_dir()) {
        s.input = p.display().to_string();
    }
    let app = App {
        s,
        ext_win: Default::default(),
        about: Default::default(),
        job: None,
        stopping: false,
        scan: None,
        tab: Tab::Log,
        filter: Default::default(),
        forensic: Default::default(),
        preview: Default::default(),
        error: None,
        confirm_close: false,
        quit: false,
        owner: Owner::default(),
    };
    match host::run(app, sys::APP_TITLE, app_icon()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            sys::message_box(&format!("화면을 열 수 없습니다: {e}\n\n명령줄에서 실행할 수 있습니다:  deid <폴더>"));
            ExitCode::FAILURE
        }
    }
}

/// 창 아이콘 (exe 아이콘과 같은 파일)
fn app_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory_with_format(include_bytes!("../../assets/icon/app.ico"), image::ImageFormat::Ico).ok()?;
    let rgba = img.to_rgba8();
    Some(egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
}

/// "라벨 [경로 입력칸] [찾아보기...]" 한 줄. 찾아보기를 누르면 true.
fn path_row(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.add_sized([64.0, 24.0], egui::Label::new(label));
        let w = (ui.available_width() - 96.0).max(120.0);
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(w));
        clicked = ui.button("찾아보기...").clicked();
    });
    clicked
}

impl App {
    /// 창이 만들어진 뒤 1회 (글꼴, 대화상자 소유 창)
    pub fn attach(&mut self, ctx: &egui::Context, window: &winit::window::Window) {
        theme::install(ctx);
        if let Ok(RawWindowHandle::Win32(h)) = window.window_handle().map(|h| h.as_raw()) {
            self.owner = Owner(h.hwnd.get());
        }
        if !self.s.input.is_empty() {
            self.tab = Tab::Log;
        }
    }

    /// 창 닫기 요청 — 검사 중이면 확인 창을 띄우고 false
    pub fn request_close(&mut self) -> bool {
        if self.running() {
            self.confirm_close = true;
            false
        } else {
            true
        }
    }

    fn running(&self) -> bool {
        self.job.is_some()
    }

    fn start(&mut self, ctx: &egui::Context) {
        let job = match self.s.job() {
            Ok(j) => j,
            Err(e) => return self.error = Some(e),
        };
        let root = std::path::absolute(&job.input).unwrap_or(job.input.clone());
        let mut scan = Scan::new(root);
        scan.log.push(format!("검사 시작: {}", job.input.display()));
        scan.log.push(format!(
            "XGBoost 맥락 탐지 {}, 임계값 {:.2}, 검사 확장자 {}/{}개{}, 사용자 정규식 {}개",
            if job.opts.ml_context { "켜짐" } else { "꺼짐" },
            job.opts.threshold,
            job.opts.exts.enabled.len(),
            crate::exts::catalog().len(),
            if job.opts.exts.scan_unknown { " + 목록 외 확장자" } else { "" },
            job.opts.custom.len()
        ));
        match worker::spawn(&job, ctx.clone()) {
            Ok(h) => {
                self.job = Some(h);
                self.stopping = false;
                self.scan = Some(scan);
                self.forensic.reset();
                if self.tab == Tab::Preview {
                    self.tab = Tab::Log;
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn stop(&mut self) {
        if let Some(h) = &mut self.job {
            h.stop();
            self.stopping = true;
        }
    }

    /// 엔진 메시지 처리
    fn poll(&mut self) {
        let (Some(h), Some(scan)) = (&self.job, &mut self.scan) else { return };
        let mut finished = false;
        while let Ok(ev) = h.rx.try_recv() {
            finished |= scan.apply(ev);
        }
        if !finished {
            return;
        }
        self.job = None; // 엔진 프로세스 정리
        self.stopping = false;
        let f = scan.finished.as_ref();
        if f.is_some_and(|f| !f.ok) {
            self.error = scan.log.last().cloned();
        } else if scan.all_records().next().is_some() && self.tab == Tab::Log {
            self.tab = Tab::Results;
        }
        if self.confirm_close {
            self.quit = true;
        }
    }

    fn handle_drop(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if let Some(p) = dropped.first() {
            if self.running() {
                return;
            }
            if p.is_dir() {
                self.s.input = p.display().to_string();
            } else {
                self.error = Some("폴더를 끌어다 놓으세요.".into());
            }
        }
    }

    fn output_dir(&self) -> Option<PathBuf> {
        self.scan.as_ref()?.finished.as_ref()?.output_dir.clone()
    }

    fn report(&self) -> Option<PathBuf> {
        self.scan.as_ref()?.finished.as_ref()?.report.clone()
    }

    // ───────────────────────────── 화면 ─────────────────────────────

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("파일", |ui| {
                if ui.add_enabled(!self.running(), egui::Button::new("검사할 폴더 열기...")).clicked() {
                    ui.close_menu();
                    if let Some(p) = sys::pick_folder(self.owner, "검사할 폴더 선택") {
                        self.s.input = p.display().to_string();
                    }
                }
                ui.separator();
                if ui.add_enabled(self.report().is_some(), egui::Button::new("보고서 열기")).clicked() {
                    ui.close_menu();
                    if let Some(p) = self.report() {
                        sys::open(&p);
                    }
                }
                if ui.add_enabled(self.output_dir().is_some(), egui::Button::new("출력 폴더 열기")).clicked() {
                    ui.close_menu();
                    if let Some(p) = self.output_dir() {
                        sys::open(&p);
                    }
                }
                ui.separator();
                if ui.button("끝내기").clicked() {
                    ui.close_menu();
                    if self.request_close() {
                        self.quit = true;
                    }
                }
            });
            ui.menu_button("보기", |ui| {
                for (t, label) in [(Tab::Log, "로그"), (Tab::Results, "검출 결과"), (Tab::Forensic, "파일 분석"), (Tab::Preview, "미리보기")] {
                    if ui.selectable_label(self.tab == t, label).clicked() {
                        self.tab = t;
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("검사할 확장자 선택...").clicked() {
                    self.ext_win.open = true;
                    ui.close_menu();
                }
            });
            ui.menu_button("도움말", |ui| {
                if ui.button("정보...").clicked() {
                    self.about.open = true;
                    ui.close_menu();
                }
            });
        });
    }

    fn top(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading(RichText::new("deid").strong().color(theme::ACCENT));
            ui.label(RichText::new("파일 속 개인정보·민감정보 비식별화").weak());
        });
        ui.add_space(2.0);
        let enabled = !self.running();
        ui.add_enabled_ui(enabled, |ui| {
            let hint = match self.s.input.trim().trim_matches('"') {
                "" => "비워 두면 <검사 폴더>_검사완료".to_string(),
                i => format!("비워 두면 {}", crate::pipeline::default_output(std::path::Path::new(i)).display()),
            };
            if path_row(ui, "검사 폴더", &mut self.s.input, "폴더를 선택하거나 창에 끌어다 놓으세요") {
                if let Some(p) = sys::pick_folder(self.owner, "검사할 폴더 선택") {
                    self.s.input = p.display().to_string();
                }
            }
            if path_row(ui, "출력 폴더", &mut self.s.output, &hint) {
                if let Some(p) = sys::pick_folder(self.owner, "출력 폴더 선택 (검사할 폴더 바깥)") {
                    self.s.output = p.display().to_string();
                }
            }
        });
        ui.add_space(4.0);
    }

    fn options(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.s;
        theme::section(ui, "검출 항목");
        let types = &InfoType::BUILTIN;
        ui.columns(2, |cols| {
            cols[0].label(RichText::new("개인정보").small().weak());
            cols[1].label(RichText::new("민감정보").small().weak());
            for (i, t) in types.iter().enumerate() {
                let col = if i < 5 { 0 } else { 1 };
                cols[col].checkbox(&mut s.enabled[i], t.label());
            }
        });
        ui.horizontal(|ui| {
            if ui.small_button("모두 선택").clicked() {
                s.enabled = [true; InfoType::BUILTIN.len()];
            }
            if ui.small_button("모두 해제").clicked() {
                s.enabled = [false; InfoType::BUILTIN.len()];
            }
        });

        theme::section(ui, "탐지 엔진");
        ui.checkbox(&mut s.ml, "XGBoost 맥락 탐지").on_hover_text(
            "기호가 바뀌거나 빠진 번호(192-168-0-1, 0109999-1234 등)를 맥락으로 추론합니다.\n꺼도 정규식·사전 규칙과 010+8자리 기계적 치환은 동작합니다.",
        );
        ui.horizontal(|ui| {
            ui.label("채택 확률");
            ui.add(egui::Slider::new(&mut s.threshold, 0.5..=0.99).step_by(0.01).fixed_decimals(2))
                .on_hover_text("이 확률 이상이면 확정 치환, 0.5 ~ 이 값 사이는 치환 후 '검토필요'로 표시 (기본 0.70)");
        });

        theme::section(ui, "그림 (OCR)");
        ui.checkbox(&mut s.ocr, "그림 속 글자 검사 (Windows 내장 OCR) — 찾은 정보는 흐리게");
        ui.add_enabled_ui(s.ocr, |ui| {
            egui::Grid::new("ocr_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("확대 배율");
                ui.add(egui::TextEdit::singleline(&mut s.ocr_scales).desired_width(80.0)).on_hover_text("작은 글자 인식용 확대 배율. 쉼표로 여러 개 (앞쪽 우선)");
                ui.end_row();
                ui.label("화소 상한");
                ui.add(egui::DragValue::new(&mut s.ocr_mp).range(1.0..=200.0).speed(0.5).suffix(" 백만"));
                ui.end_row();
                ui.label("흐림 강도");
                ui.add(egui::Slider::new(&mut s.blur, 0.1..=10.0).step_by(0.1).fixed_decimals(1));
                ui.end_row();
            });
        });

        theme::section(ui, "파일");
        ui.checkbox(&mut s.rename, "파일·폴더 이름의 정보도 비식별화");
        ui.checkbox(&mut s.scan_exe, "실행 파일(exe, dll)도 검사");
        egui::Grid::new("file_grid").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("최대 파일 크기");
            ui.add(egui::DragValue::new(&mut s.max_mb).range(1..=65536).suffix(" MB"));
            ui.end_row();
            ui.label("바이너리 문자열 최소 길이");
            ui.add(egui::DragValue::new(&mut s.min_run).range(3..=64));
            ui.end_row();
        });
        let label = format!("검사할 확장자 선택 ({}/{})...", s.exts.enabled.len(), crate::exts::catalog().len());
        if ui.button(label).clicked() {
            self.ext_win.open = true;
        }

        theme::section(ui, "사용자 정의 정규식");
        ui.label(RichText::new("한 줄에 하나씩  라벨=정규식   (그룹1이 있으면 그룹1만 치환, #은 주석)").small().weak());
        ui.add(
            egui::TextEdit::multiline(&mut self.s.patterns)
                .hint_text("사번=EMP-\\d{6}\n차량번호=\\d{2,3}[가-힣]\\d{4}")
                .font(egui::TextStyle::Monospace)
                .desired_rows(4)
                .desired_width(f32::INFINITY),
        );
        ui.horizontal(|ui| {
            if ui.button("파일 불러오기...").clicked() {
                let filters: [(&str, &[&str]); 2] = [("패턴 파일", &["txt", "pat", "cfg"]), ("모든 파일", &["*"])];
                if let Some(p) = sys::pick_file(self.owner, "사용자 정의 정규식 파일 (줄마다 라벨=정규식)", &filters) {
                    match std::fs::read(&p) {
                        Ok(b) => {
                            let t = crate::common::decode_text(&b).map(|(t, _)| t).unwrap_or_else(|| String::from_utf8_lossy(&b).into_owned());
                            let cur = self.s.patterns.trim_end();
                            self.s.patterns = if cur.is_empty() { t } else { format!("{cur}\n{t}") };
                        }
                        Err(e) => self.error = Some(format!("파일을 읽을 수 없습니다: {e}")),
                    }
                }
            }
            if ui.button("미리보기에서 시험").clicked() {
                self.tab = Tab::Preview;
            }
        });
        for e in self.s.pattern_errors() {
            ui.colored_label(ui.visuals().error_fg_color, RichText::new(e).small());
        }
        ui.add_space(8.0);
        if ui.small_button("모든 설정 기본값으로").clicked() {
            let keep = (self.s.input.clone(), self.s.output.clone(), self.s.patterns.clone());
            self.s = Settings { input: keep.0, output: keep.1, patterns: keep.2, ..Default::default() };
        }
    }

    fn bottom(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let (frac, text, preparing) = match &self.scan {
            Some(s) if s.running() => match s.total {
                Some(t) if t > 0 => (s.done as f32 / t as f32, format!("{} / {t}", s.done), false),
                _ => (0.0, "준비 중".into(), true),
            },
            Some(s) if s.finished.as_ref().is_some_and(|f| f.ok) => (1.0, "완료".into(), false),
            _ => (0.0, String::new(), false),
        };
        if frac > 0.0 || preparing {
            ui.add(egui::ProgressBar::new(frac).text(text).fill(theme::ACCENT).animate(preparing));
        } else {
            // 0% 막대는 egui 가 둥근 점을 그리므로 빈 홈만 그린다
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 9.0, ui.visuals().extreme_bg_color);
        }
        let status = match &self.scan {
            _ if self.stopping => "중지하는 중... (처리 중인 파일이 끝나면 멈춥니다)".to_string(),
            Some(s) => s.status_line(),
            None => "준비".into(),
        };
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(status).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let running = self.running();
                let (label, fill) = if running { ("중지", ui.visuals().error_fg_color) } else { ("검사 시작", theme::ACCENT) };
                let btn = egui::Button::new(RichText::new(label).strong().color(egui::Color32::WHITE)).fill(fill).min_size(egui::vec2(110.0, 30.0));
                if ui.add_enabled(!self.stopping, btn).clicked() {
                    if running {
                        self.stop();
                    } else {
                        self.start(ui.ctx());
                    }
                }
                let out = self.output_dir().filter(|_| !running);
                if ui.add_enabled(out.is_some(), egui::Button::new("출력 폴더 열기")).clicked() {
                    if let Some(p) = &out {
                        sys::open(p);
                    }
                }
                let rep = self.report().filter(|_| !running);
                if ui.add_enabled(rep.is_some(), egui::Button::new("보고서 열기")).clicked() {
                    if let Some(p) = &rep {
                        sys::open(p);
                    }
                }
            });
        });
        ui.add_space(6.0);
    }

    fn center(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Log, "로그");
            let (n, review) = self.scan.as_ref().map_or((0, 0), |s| (s.all_records().count(), s.all_records().filter(|r| r.review).count()));
            let t = if review > 0 { format!("검출 결과 ({n}, 검토 {review})") } else { format!("검출 결과 ({n})") };
            ui.selectable_value(&mut self.tab, Tab::Results, t);
            let files = self.scan.as_ref().map_or(0, |s| s.entries.len());
            ui.selectable_value(&mut self.tab, Tab::Forensic, format!("파일 분석 ({files})"));
            ui.selectable_value(&mut self.tab, Tab::Preview, "미리보기");
        });
        ui.separator();
        match self.tab {
            Tab::Log => views::log(ui, self.scan.as_ref().map_or(&[][..], |s| &s.log[..])),
            Tab::Results => views::results(ui, self.scan.as_ref(), &mut self.filter),
            Tab::Forensic => self.forensic.ui(ui, self.scan.as_ref()),
            Tab::Preview => views::preview(ui, &mut self.preview, &self.s),
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(e) = self.error.clone() {
            let mut close = false;
            egui::Window::new("알림").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(e);
                ui.add_space(6.0);
                close = ui.button("   확인   ").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape));
            });
            if close {
                self.error = None;
            }
        }
        if self.confirm_close {
            egui::Window::new("닫기").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                if self.stopping {
                    ui.label("중지하는 중... 처리 중인 파일이 끝나면 닫힙니다.");
                    return;
                }
                ui.label("검사 중입니다. 중지하고 닫을까요?\n(출력 폴더는 불완전한 상태로 남습니다)");
                ui.horizontal(|ui| {
                    if ui.button("중지하고 닫기").clicked() {
                        self.stop();
                    }
                    if ui.button("계속 검사").clicked() {
                        self.confirm_close = false;
                    }
                });
            });
        }
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        self.poll();
        self.handle_drop(ctx);
        let enabled = !self.running();

        egui::TopBottomPanel::top("menu").show(ctx, |ui| self.menu(ui));
        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top(ui));
        egui::TopBottomPanel::bottom("bottom").show(ctx, |ui| self.bottom(ui));
        egui::SidePanel::left("options").resizable(true).default_width(330.0).width_range(280.0..=520.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_enabled_ui(enabled, |ui| self.options(ui));
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| self.center(ui));
        self.ext_win.show(ctx, &mut self.s.exts, enabled);
        self.about.show(ctx);
        self.dialogs(ctx);
    }
}

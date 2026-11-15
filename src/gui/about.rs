//! 정보 창 — 프로그램·개발자 정보와 사용한 오픈소스 목록.
//! 목록(`assets/licenses.tsv`)은 `python assets/make_licenses.py` 로 Cargo 의존성 그래프에서 생성한다.

use egui::RichText;

pub const GITHUB: &str = "https://github.com/maspo1";
const LICENSES: &str = include_str!("../../assets/licenses.tsv");

/// (이름, 버전, 라이선스, 저장소, 용도)
pub(crate) fn crates() -> impl Iterator<Item = [&'static str; 5]> {
    LICENSES.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).map(|l| {
        let mut it = l.split('\t');
        std::array::from_fn(|_| it.next().unwrap_or(""))
    })
}

/// 실행 시 쓰는 OS 기능과 글꼴 (Cargo 의존성이 아닌 것)
pub(crate) const RUNTIME: &[(&str, &str, &str)] = &[
    ("Windows.Media.Ocr", "Windows 10/11 내장 OCR", "Microsoft — OS 구성 요소"),
    ("Vision (VNRecognizeTextRequest)", "macOS 내장 OCR", "Apple — OS 구성 요소"),
    ("맑은 고딕 / Apple SD 산돌고딕 Neo", "화면 한글 글꼴 (OS 설치본을 읽기만 함, 배포 안 함)", "Microsoft / Apple — OS 구성 요소"),
    ("Ubuntu Light", "egui 기본 글꼴", "Ubuntu Font Licence 1.0"),
    ("Hack", "egui 기본 고정폭 글꼴", "MIT / Bitstream Vera"),
    ("Noto Emoji", "egui 기본 그림 문자", "SIL OFL 1.1"),
    ("emoji-icon-font", "egui 기본 아이콘", "MIT"),
];

#[derive(Default)]
pub struct About {
    pub open: bool,
    search: String,
}

impl About {
    pub fn show(&mut self, ctx: &egui::Context) {
        let mut open = self.open;
        egui::Window::new("deid 정보").open(&mut open).collapsible(false).default_size([560.0, 520.0]).show(ctx, |ui| self.body(ui));
        self.open = open;
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("deid").strong().color(super::theme::ACCENT));
            ui.label(format!("버전 {}", env!("CARGO_PKG_VERSION")));
        });
        ui.label("파일 속 개인정보·민감정보 비식별화 도구 — 순수 Rust, 단일 실행 파일");
        egui::Grid::new("about_info").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            ui.label(RichText::new("개발자").weak());
            ui.hyperlink_to("github.com/maspo1", GITHUB);
            ui.end_row();
            ui.label(RichText::new("화면").weak());
            ui.label("egui (CPU 렌더링 — 그래픽 드라이버 미사용)");
            ui.end_row();
            ui.label(RichText::new("검사 엔진").weak());
            ui.label("별도 프로세스 (검사할 때만 실행, 끝나면 메모리 반환)");
            ui.end_row();
        });
        ui.separator();
        let list: Vec<[&str; 5]> = crates().collect();
        ui.horizontal(|ui| {
            ui.strong(format!("사용한 오픈소스 ({}개)", list.len()));
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("이름·라이선스 검색").desired_width(160.0));
        });
        let q = self.search.trim().to_lowercase();
        egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 4.0).show(ui, |ui| {
            egui::Grid::new("about_crates").num_columns(4).striped(true).spacing([10.0, 2.0]).show(ui, |ui| {
                for h in ["이름", "버전", "라이선스", "용도"] {
                    ui.strong(h);
                }
                ui.end_row();
                for [name, ver, lic, repo, usage] in list.iter().filter(|c| q.is_empty() || c[0].contains(&q) || c[2].to_lowercase().contains(&q)) {
                    if repo.is_empty() {
                        ui.label(*name);
                    } else {
                        ui.hyperlink_to(*name, *repo);
                    }
                    ui.label(*ver);
                    ui.label(*lic);
                    ui.label(RichText::new(*usage).small().weak());
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.strong("실행 시 사용하는 OS 기능·글꼴");
            egui::Grid::new("about_runtime").num_columns(3).striped(true).spacing([10.0, 2.0]).show(ui, |ui| {
                for (a, b, c) in RUNTIME {
                    ui.label(*a);
                    ui.label(RichText::new(*b).small());
                    ui.label(RichText::new(*c).small().weak());
                    ui.end_row();
                }
            });
        });
    }
}

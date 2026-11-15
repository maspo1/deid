//! 검사 1회의 결과 모음 — 로그, 검출 결과 표, 포렌식 보기가 모두 이것을 읽는다 (복사본 없음).

use crate::ipc::{relative, Event, FileEntry, NameEntry, Rec, Status};
use std::path::PathBuf;
use std::time::Instant;

pub struct Finished {
    pub ok: bool,
    pub output_dir: Option<PathBuf>,
    pub report: Option<PathBuf>,
}

pub struct Scan {
    pub root: PathBuf,
    pub entries: Vec<FileEntry>,
    pub names: Vec<NameEntry>,
    pub log: Vec<String>,
    pub total: Option<usize>,
    pub done: usize,
    pub started: Instant,
    pub finished: Option<Finished>,
    /// 내용이 바뀔 때마다 증가 (보기별 캐시 무효화)
    pub version: u64,
}

/// 검출 1건의 위치
#[derive(Clone, Copy)]
pub enum Src {
    File(usize),
    Name(usize),
}

impl Scan {
    pub fn new(root: PathBuf) -> Self {
        Scan { root, entries: vec![], names: vec![], log: vec![], total: None, done: 0, started: Instant::now(), finished: None, version: 0 }
    }

    pub fn rel(&self, p: &std::path::Path) -> String {
        relative(p, &self.root)
    }

    pub fn records(&self, s: Src) -> &[Rec] {
        match s {
            Src::File(i) => &self.entries[i].records,
            Src::Name(i) => &self.names[i].records,
        }
    }

    pub fn path(&self, s: Src) -> &std::path::Path {
        match s {
            Src::File(i) => &self.entries[i].path,
            Src::Name(i) => &self.names[i].path,
        }
    }

    pub fn out_path(&self, s: Src) -> Option<&std::path::Path> {
        match s {
            Src::File(i) => Some(&self.entries[i].out),
            Src::Name(_) => self.finished.as_ref().and_then(|f| f.output_dir.as_deref()),
        }
    }

    pub fn all_records(&self) -> impl Iterator<Item = &Rec> {
        self.entries.iter().flat_map(|e| &e.records).chain(self.names.iter().flat_map(|n| &n.records))
    }

    pub fn running(&self) -> bool {
        self.finished.is_none()
    }

    /// 엔진 메시지 반영. 끝났으면 true.
    pub fn apply(&mut self, ev: Event) -> bool {
        self.version += 1;
        match ev {
            Event::Collected(n) => self.total = Some(n),
            Event::File { done, total, entry } => {
                self.done = done;
                self.total = Some(total);
                self.log_file(&entry);
                self.entries.push(entry);
            }
            Event::Name(n) => self.names.push(n),
            Event::Done { output_dir, report } => {
                self.entries.sort_by(|a, b| a.path.cmp(&b.path));
                let secs = self.started.elapsed().as_secs_f32();
                let recs = self.all_records().count();
                let review = self.all_records().filter(|r| r.review).count();
                let masked = self.entries.iter().filter(|e| e.status == Status::Masked).count();
                let rv = if review > 0 { format!(", 검토필요 {review}건") } else { String::new() };
                self.log.push(format!(
                    "완료 — 파일 {}개, 치환 {masked}개, 검출 {recs}건{rv}, 이름 변경 {}개 ({secs:.1}초)",
                    self.entries.len(),
                    self.names.len()
                ));
                if review > 0 {
                    self.log.push("※ 애매한 항목은 일단 치환했습니다. [검출 결과]·[파일 분석] 탭이나 보고서의 \"(검토필요)\" 행을 확인하세요.".into());
                }
                self.log.push(format!("출력 폴더: {}", output_dir.display()));
                self.log.push(format!("보고서  : {}", report.display()));
                self.finished = Some(Finished { ok: true, output_dir: Some(output_dir), report: Some(report) });
                return true;
            }
            Event::Failed(e) => {
                self.log.push(format!("오류: {e}"));
                self.finished = Some(Finished { ok: false, output_dir: None, report: None });
                return true;
            }
        }
        false
    }

    fn log_file(&mut self, e: &FileEntry) {
        if e.records.is_empty() && e.notes.is_empty() {
            return;
        }
        let name = self.rel(&e.path);
        self.log.push(if e.records.is_empty() {
            format!("[그대로] {name}")
        } else {
            let review = e.records.iter().filter(|x| x.review).count();
            let extra = if review > 0 { format!(", 검토필요 {review}") } else { String::new() };
            format!("[치환]   {name} — {}건{extra}", e.records.len())
        });
        self.log.extend(e.notes.iter().map(|n| format!("           - {n}")));
    }

    pub fn status_line(&self) -> String {
        match &self.finished {
            None => match self.total {
                Some(t) => format!("검사 중... {} / {t}", self.done),
                None => "파일 목록을 만드는 중...".into(),
            },
            Some(f) if f.ok => self.log.iter().rev().find(|l| l.starts_with("완료")).cloned().unwrap_or_default(),
            Some(_) => "중단됨".into(),
        }
    }
}

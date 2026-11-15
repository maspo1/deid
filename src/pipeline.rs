//! 폴더 단위 처리: `애플` → `애플_검사완료` (폴더 구조 동일 유지) + CSV 보고서
//! 파일·폴더 이름에 정보가 있으면 이름만 비식별화한다 (계층 구조는 그대로).

use crate::handlers::{process, Ctx};
use crate::common::Record;
use crate::exts::Decision;
use crate::naming;
use std::collections::HashSet;
use crate::report::{write_csv, ReportRow};
use crate::rules::Detector;
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub const OUTPUT_SUFFIX: &str = "_검사완료";
pub const REPORT_NAME: &str = "검사보고서.csv";

pub struct FileResult {
    /// 원본 경로
    pub path: PathBuf,
    /// 출력 경로 (이름이 비식별화됐을 수 있음)
    pub out_path: PathBuf,
    pub kind: String,
    /// 원본 크기 (바이트)
    pub size: u64,
    pub records: Vec<Record>,
    pub modified: bool,
    pub notes: Vec<String>,
}

/// 이름이 바뀐 파일/폴더
pub struct Renamed {
    pub path: PathBuf,
    pub is_dir: bool,
    pub records: Vec<Record>,
}

pub struct Summary {
    pub output_dir: PathBuf,
    pub report: PathBuf,
    pub files: Vec<FileResult>,
    pub renamed: Vec<Renamed>,
}

/// `\\?\` 접두어 없는 절대 경로
fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

pub fn default_output(input: &Path) -> PathBuf {
    let input = absolute(input);
    let name = input.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "output".into());
    input.with_file_name(format!("{name}{OUTPUT_SUFFIX}"))
}

fn process_file(src: &Path, dst: &Path, det: &Detector) -> FileResult {
    let size = fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    let mut r = FileResult { path: src.to_path_buf(), out_path: dst.to_path_buf(), kind: String::new(), size, records: vec![], modified: false, notes: vec![] };
    let copy = |r: &mut FileResult| {
        if let Err(e) = fs::copy(src, dst) {
            r.notes.push(format!("복사 실패: {e}"));
        }
    };
    if size > det.opts.max_file_size {
        r.notes.push(format!("크기 제한 초과({size} bytes) - 원본 복사(미검사)"));
        copy(&mut r);
        return r;
    }
    let bytes = match fs::read(src) {
        Ok(b) => b,
        Err(e) => {
            r.notes.push(format!("읽기 실패: {e}"));
            return r;
        }
    };
    let name = src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    // 확장자 선택(고급 옵션) — 실제 형식은 매직바이트로 보고, 위장 파일은 체크 해제돼도 검사
    let kind = crate::filetype::detect(&bytes);
    let mut pre_notes = Vec::new();
    match det.opts.exts.decide(&name, &kind) {
        Decision::Inspect => {}
        Decision::Disguised(n) => pre_notes.push(n),
        Decision::Skip(n) => {
            r.kind = kind.label();
            r.notes.push(n);
            copy(&mut r);
            return r;
        }
    }
    let o = process(&name, &bytes, &Ctx::new(det));
    r.kind = o.kind;
    r.records = o.records;
    r.notes = pre_notes.into_iter().chain(o.notes).collect();
    match o.data {
        Some(d) => match fs::write(dst, d) {
            Ok(_) => r.modified = true,
            Err(e) => r.notes.push(format!("쓰기 실패: {e}")),
        },
        None => copy(&mut r),
    }
    r
}

/// 하위 폴더를 재귀적으로 복제하며 (원본, 대상) 파일 목록 수집. 심볼릭 링크는 따라가지 않는다.
fn collect(
    src: &Path,
    dst: &Path,
    det: &Detector,
    jobs: &mut Vec<(PathBuf, PathBuf)>,
    renamed: &mut Vec<Renamed>,
) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let mut entries: Vec<_> = fs::read_dir(src)
        .map_err(|e| format!("{}: {e}", src.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    let mut taken = HashSet::new();
    for e in entries {
        let Ok(ft) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        let new_name = match naming::deidentify(&name, det) {
            Some((n, records)) => {
                renamed.push(Renamed { path: e.path(), is_dir: ft.is_dir(), records });
                n
            }
            None => name,
        };
        let target = dst.join(naming::unique(&new_name, &mut taken));
        if ft.is_dir() {
            collect(&e.path(), &target, det, jobs, renamed)?;
        } else if ft.is_file() {
            jobs.push((e.path(), target));
        }
    }
    Ok(())
}

/// 진행 상황 알림 (GUI 진행 막대·로그용)
pub enum Event<'a> {
    /// 파일 목록 수집 완료
    Collected { files: usize },
    /// 파일 1개 처리 완료 (병렬 처리이므로 순서는 보장되지 않음)
    File { done: usize, total: usize, result: &'a FileResult },
}

pub fn run(input: &Path, output: Option<&Path>, det: &Detector) -> Result<Summary, String> {
    run_with(input, output, det, &|_| {}, &AtomicBool::new(false))
}

/// 진행 알림 + 중지 지원 실행. `cancel` 이 켜지면 남은 파일을 건너뛰고 Err 를 돌려준다.
pub fn run_with(
    input: &Path,
    output: Option<&Path>,
    det: &Detector,
    on_event: &(dyn Fn(Event) + Sync),
    cancel: &AtomicBool,
) -> Result<Summary, String> {
    let input = absolute(input);
    if !input.is_dir() {
        return Err(format!("입력 폴더가 없습니다: {}", input.display()));
    }
    let out = output.map(absolute).unwrap_or_else(|| default_output(&input));
    if out == input || out.starts_with(&input) {
        return Err("출력 폴더는 입력 폴더 바깥이어야 합니다".into());
    }
    fs::create_dir_all(&out).map_err(|e| format!("출력 폴더 생성 실패: {e}"))?;

    // 폴더 구조 복제 + 파일 목록
    let mut jobs = Vec::new();
    let mut renamed = Vec::new();
    collect(&input, &out, det, &mut jobs, &mut renamed)?;

    let total = jobs.len();
    on_event(Event::Collected { files: total });
    let done = AtomicUsize::new(0);
    let mut files: Vec<FileResult> = jobs
        .par_iter()
        .filter_map(|(s, d)| {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let r = process_file(s, d, det);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            on_event(Event::File { done: n, total, result: &r });
            Some(r)
        })
        .collect();
    if cancel.load(Ordering::Relaxed) {
        return Err(format!("사용자가 중지했습니다 ({}/{total}개 처리됨, 출력 폴더는 불완전합니다)", files.len()));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut rows: Vec<ReportRow> = renamed
        .iter()
        .flat_map(|n| {
            let tag = if n.is_dir { "폴더명" } else { "파일명" };
            n.records.iter().map(move |r| ReportRow {
                original: r.original.clone(),
                masked: r.masked.clone(),
                kind: format!("{}({tag})", r.label()),
                path: n.path.display().to_string(),
            })
        })
        .collect();
    rows.extend(files
        .iter()
        .flat_map(|f| {
            f.records.iter().map(move |r| ReportRow {
                original: r.original.clone(),
                masked: r.masked.clone(),
                kind: r.label(),
                path: f.path.display().to_string(),
            })
        }));
    let report = out.join(REPORT_NAME);
    write_csv(&report, &rows).map_err(|e| format!("보고서 쓰기 실패: {e}"))?;
    Ok(Summary { output_dir: out, report, files, renamed })
}

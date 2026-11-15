//! 엔진 프로세스 분리 — 크롬·엣지·웹뷰가 화면 프로세스와 작업 프로세스를 나누는 것과 같은 구조.
//!
//! * 화면(GUI) 프로세스: 창과 결과 목록만 가진다. 평소 메모리가 작게 유지된다.
//! * 엔진 프로세스 (`deid --engine`): 검사할 때만 같은 exe 로 띄우고, 끝나면 종료 → 정규식·모델·OCR·
//!   파일 버퍼·스레드 풀이 쓰던 메모리가 OS 에 모두 반환된다. 문서 파서가 비정상 종료돼도 화면은 살아 있다.
//!
//! 통신: 표준 입출력의 한 줄 = 한 메시지, 필드는 탭 구분 (`esc` 로 탭·줄바꿈·역슬래시 이스케이프)
//! * 화면 → 엔진 (stdin): 작업 설정 `키\t값` 줄들 + `END`, 이후 중지 요청은 `STOP`
//! * 엔진 → 화면 (stdout): `C` 파일 수, `F` 파일 결과(+ `N` 알림, `R` 검출) `.` 끝, `M` 이름 검출, `D` 완료, `X` 실패

use crate::common::{InfoType, Record};
use crate::config::Options;
use crate::pipeline::{self, Event as PEvent, FileResult};
use crate::rules::Detector;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ───────────────────────────── 이스케이프 ─────────────────────────────

pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

pub fn unesc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('t') => o.push('\t'),
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            Some(x) => o.push(x),
            None => o.push('\\'),
        }
    }
    o
}

fn fields(line: &str) -> Vec<String> {
    line.split('\t').map(unesc).collect()
}

// ───────────────────────────── 작업 설정 ─────────────────────────────

pub struct Job {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub opts: Options,
}

fn b(x: bool) -> &'static str {
    if x {
        "1"
    } else {
        "0"
    }
}

pub fn encode_job(j: &Job) -> String {
    let o = &j.opts;
    let mut lines: Vec<(String, String)> = vec![
        ("input".into(), j.input.display().to_string()),
        ("output".into(), j.output.as_ref().map(|p| p.display().to_string()).unwrap_or_default()),
        ("enabled".into(), o.enabled.iter().map(|t| t.key().to_string()).collect::<Vec<_>>().join(",")),
        ("threshold".into(), o.threshold.to_string()),
        ("ml".into(), b(o.ml_context).into()),
        ("ocr".into(), b(o.ocr).into()),
        ("ocr_scales".into(), o.ocr_scales.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")),
        ("ocr_max_pixels".into(), o.ocr_max_pixels.to_string()),
        ("ocr_enhance".into(), b(o.ocr_enhance).into()),
        ("depth".into(), o.depth.to_string()),
        ("blur".into(), o.blur_strength.to_string()),
        ("rename".into(), b(o.rename_files).into()),
        ("scan_exe".into(), b(o.scan_executables).into()),
        ("max_file_size".into(), o.max_file_size.to_string()),
        ("min_run".into(), o.min_binary_run.to_string()),
        ("max_depth".into(), o.max_depth.to_string()),
        ("exts".into(), {
            let mut v: Vec<&str> = o.exts.enabled.iter().map(String::as_str).collect();
            v.sort();
            v.join(",")
        }),
        ("scan_unknown".into(), b(o.exts.scan_unknown).into()),
        ("check_disguised".into(), b(o.exts.check_disguised).into()),
    ];
    for c in &o.custom {
        lines.push(("pattern".into(), format!("{}={}", c.label, c.regex.as_str())));
    }
    for v in &o.excluded {
        lines.push(("exclude".into(), v.clone()));
    }
    for v in &o.excluded_formats {
        lines.push(("exclude_format".into(), v.clone()));
    }
    let mut s: String = lines.iter().map(|(k, v)| format!("{k}\t{}\n", esc(v))).collect();
    s.push_str("END\n");
    s
}

fn list(v: &str) -> impl Iterator<Item = &str> {
    v.split(',').map(str::trim).filter(|x| !x.is_empty())
}

pub fn decode_job(lines: &[String]) -> Result<Job, String> {
    let mut o = Options::default();
    let (mut input, mut output) = (None, None);
    for line in lines {
        let (k, v) = line.split_once('\t').ok_or_else(|| format!("작업 설정 형식 오류: {line}"))?;
        let v = unesc(v);
        let num = |what: &str| format!("작업 설정 {what} 값 오류: {v}");
        match k {
            "input" => input = Some(PathBuf::from(&v)),
            "output" => output = (!v.is_empty()).then(|| PathBuf::from(&v)),
            "enabled" => o.enabled = list(&v).filter_map(InfoType::from_key).collect(),
            "threshold" => o.threshold = v.parse().map_err(|_| num(k))?,
            "ml" => o.ml_context = v == "1",
            "ocr" => o.ocr = v == "1",
            "ocr_scales" => o.ocr_scales = list(&v).filter_map(|x| x.parse().ok()).collect(),
            "ocr_max_pixels" => o.ocr_max_pixels = v.parse().map_err(|_| num(k))?,
            "ocr_enhance" => o.ocr_enhance = v == "1",
            "depth" => o.depth = v.parse().map_err(|_| num(k))?,
            "blur" => o.blur_strength = v.parse().map_err(|_| num(k))?,
            "rename" => o.rename_files = v == "1",
            "scan_exe" => o.scan_executables = v == "1",
            "max_file_size" => o.max_file_size = v.parse().map_err(|_| num(k))?,
            "min_run" => o.min_binary_run = v.parse().map_err(|_| num(k))?,
            "max_depth" => o.max_depth = v.parse().map_err(|_| num(k))?,
            "exts" => o.exts.enabled = list(&v).map(str::to_string).collect(),
            "scan_unknown" => o.exts.scan_unknown = v == "1",
            "check_disguised" => o.exts.check_disguised = v == "1",
            "pattern" => o.add_custom(&v)?,
            "exclude" => {
                o.excluded.insert(v);
            }
            "exclude_format" => {
                o.excluded_formats.insert(v);
            }
            _ => {} // 이후 버전의 키는 무시
        }
    }
    Ok(Job { input: input.ok_or("작업 설정에 input 이 없습니다")?, output, opts: o })
}

// ───────────────────────────── 결과 메시지 ─────────────────────────────

/// 파일 상태 (포렌식 보기의 색상·필터 기준)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// 정보를 찾아 치환한 결과 파일을 씀
    Masked,
    /// 정보를 찾았지만 원본 형식을 유지해야 해서 복사만 함 (보고서에만 기록)
    Found,
    /// 정보 없음
    Clean,
    /// 확장자 선택·크기 제한으로 검사하지 않음
    Skipped,
    /// 읽기·쓰기 실패
    Error,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Masked => "치환",
            Status::Found => "검출",
            Status::Clean => "정상",
            Status::Skipped => "제외",
            Status::Error => "오류",
        }
    }
    fn code(self) -> char {
        match self {
            Status::Masked => 'M',
            Status::Found => 'F',
            Status::Clean => 'C',
            Status::Skipped => 'S',
            Status::Error => 'E',
        }
    }
    fn from_code(c: &str) -> Status {
        match c {
            "M" => Status::Masked,
            "F" => Status::Found,
            "S" => Status::Skipped,
            "E" => Status::Error,
            _ => Status::Clean,
        }
    }
    pub fn of(r: &FileResult) -> Status {
        let note = |k: &str| r.notes.iter().any(|n| n.contains(k));
        if note("실패") {
            Status::Error
        } else if r.modified {
            Status::Masked
        } else if !r.records.is_empty() {
            Status::Found
        } else if note("원본 복사") || note("미검사") {
            Status::Skipped
        } else {
            Status::Clean
        }
    }
}

/// 검출 1건 (보고서 1행)
#[derive(Clone, Debug)]
pub struct Rec {
    pub original: String,
    pub masked: String,
    /// "IP주소" / "IP주소(검토필요)" / "이메일주소(파일명)"
    pub label: String,
    pub review: bool,
}

impl Rec {
    fn from(r: &Record, label: String) -> Rec {
        Rec { original: r.original.clone(), masked: r.masked.clone(), label, review: r.review }
    }
    /// 검토 표시를 뺀 정보 타입 이름
    pub fn base(&self) -> &str {
        self.label.split('(').next().unwrap_or(&self.label)
    }
}

/// 파일 1개의 검사 결과
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub out: PathBuf,
    pub kind: String,
    pub size: u64,
    pub status: Status,
    pub notes: Vec<String>,
    pub records: Vec<Rec>,
}

/// 이름(파일·폴더명)에서 나온 검출
#[derive(Clone, Debug)]
pub struct NameEntry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub records: Vec<Rec>,
}

pub enum Event {
    Collected(usize),
    File { done: usize, total: usize, entry: FileEntry },
    Name(NameEntry),
    Done { output_dir: PathBuf, report: PathBuf },
    Failed(String),
}

fn write_recs(s: &mut String, recs: &[Rec]) {
    for r in recs {
        s.push_str(&format!("R\t{}\t{}\t{}\t{}\n", esc(&r.original), esc(&r.masked), esc(&r.label), b(r.review)));
    }
}

fn encode_file(done: usize, total: usize, r: &FileResult) -> String {
    let mut s = format!(
        "F\t{done}\t{total}\t{}\t{}\t{}\t{}\t{}\n",
        esc(&r.path.display().to_string()),
        esc(&r.out_path.display().to_string()),
        esc(&r.kind),
        r.size,
        Status::of(r).code()
    );
    for n in &r.notes {
        s.push_str(&format!("N\t{}\n", esc(n)));
    }
    let recs: Vec<Rec> = r.records.iter().map(|x| Rec::from(x, x.label())).collect();
    write_recs(&mut s, &recs);
    s.push_str(".\n");
    s
}

/// 엔진 출력 해석기 — 줄을 넣으면 완성된 메시지를 돌려준다
#[derive(Default)]
pub struct Parser {
    file: Option<(usize, usize, FileEntry)>,
    name: Option<NameEntry>,
}

impl Parser {
    pub fn feed(&mut self, line: &str) -> Option<Event> {
        let f = fields(line);
        let get = |i: usize| f.get(i).cloned().unwrap_or_default();
        match f.first().map(String::as_str) {
            Some("C") => Some(Event::Collected(get(1).parse().unwrap_or(0))),
            Some("F") => {
                let e = FileEntry {
                    path: get(3).into(),
                    out: get(4).into(),
                    kind: get(5),
                    size: get(6).parse().unwrap_or(0),
                    status: Status::from_code(&get(7)),
                    notes: vec![],
                    records: vec![],
                };
                self.file = Some((get(1).parse().unwrap_or(0), get(2).parse().unwrap_or(0), e));
                None
            }
            Some("N") => {
                if let Some((_, _, e)) = &mut self.file {
                    e.notes.push(get(1));
                }
                None
            }
            Some("R") => {
                let r = Rec { original: get(1), masked: get(2), label: get(3), review: get(4) == "1" };
                if let Some((_, _, e)) = &mut self.file {
                    e.records.push(r);
                } else if let Some(n) = &mut self.name {
                    n.records.push(r);
                }
                None
            }
            Some(".") => {
                if let Some((done, total, entry)) = self.file.take() {
                    return Some(Event::File { done, total, entry });
                }
                self.name.take().map(Event::Name)
            }
            Some("M") => {
                self.name = Some(NameEntry { path: get(1).into(), is_dir: get(2) == "1", records: vec![] });
                None
            }
            Some("D") => Some(Event::Done { output_dir: get(1).into(), report: get(2).into() }),
            Some("X") => Some(Event::Failed(get(1))),
            _ => None,
        }
    }
}

// ───────────────────────────── 엔진 프로세스 ─────────────────────────────

/// `deid --engine` — stdin 으로 작업을 받고 stdout 으로 진행 상황을 보낸다
pub fn engine_main() -> ExitCode {
    let stdin = std::io::stdin();
    let mut lines = Vec::new();
    let mut input = stdin.lock().lines();
    for l in input.by_ref() {
        let Ok(l) = l else { break };
        if l == "END" {
            break;
        }
        lines.push(l);
    }
    let out = Arc::new(Mutex::new(std::io::stdout()));
    let send = |s: &str| {
        let mut o = out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = o.write_all(s.as_bytes());
        let _ = o.flush();
    };
    let job = match decode_job(&lines) {
        Ok(j) => j,
        Err(e) => {
            send(&format!("X\t{}\n", esc(&e)));
            return ExitCode::FAILURE;
        }
    };
    // 중지 요청 (또는 화면 프로세스 종료로 stdin 이 닫힘) → 남은 파일 건너뜀
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            for l in std::io::stdin().lock().lines() {
                match l {
                    Ok(l) if l.trim() == "STOP" => break,
                    Ok(_) => continue,
                    Err(_) => break,
                }
            }
            cancel.store(true, Ordering::Relaxed);
        });
    }
    let det = Detector::new(job.opts);
    let on_event = |ev: PEvent| match ev {
        PEvent::Collected { files } => send(&format!("C\t{files}\n")),
        PEvent::File { done, total, result } => send(&encode_file(done, total, result)),
    };
    match pipeline::run_with(&job.input, job.output.as_deref(), &det, &on_event, &cancel) {
        Ok(s) => {
            let mut m = String::new();
            for n in &s.renamed {
                m.push_str(&format!("M\t{}\t{}\n", esc(&n.path.display().to_string()), b(n.is_dir)));
                let tag = if n.is_dir { "폴더명" } else { "파일명" };
                let recs: Vec<Rec> = n.records.iter().map(|r| Rec::from(r, format!("{}({tag})", r.label()))).collect();
                write_recs(&mut m, &recs);
                m.push_str(".\n");
            }
            send(&m);
            send(&format!("D\t{}\t{}\n", esc(&s.output_dir.display().to_string()), esc(&s.report.display().to_string())));
            ExitCode::SUCCESS
        }
        Err(e) => {
            send(&format!("X\t{}\n", esc(&e)));
            ExitCode::FAILURE
        }
    }
}

/// 실행 중인 exe 를 엔진 모드로 띄운다
pub fn spawn_engine(job: &Job) -> std::io::Result<std::process::Child> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("--engine").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(encode_job(job).as_bytes())?;
        stdin.flush()?;
    }
    Ok(child)
}

pub fn relative(p: &Path, root: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_roundtrip() {
        for s in ["a\tb\nc\\d\re", "", "\\", "끝\\t"] {
            assert_eq!(unesc(&esc(s)), s);
            assert!(!esc(s).contains(['\t', '\n']));
        }
    }

    #[test]
    fn job_roundtrip() {
        let mut o = Options::default();
        o.ml_context = false;
        o.threshold = 0.8;
        o.exts.enabled.remove("log");
        o.add_custom("사번=EMP-(\\d{6})\t").unwrap();
        o.enabled.remove(&InfoType::Comment);
        o.excluded.insert("010-1234-5678".into());
        o.excluded.insert("탭\t포함".into());
        o.excluded_formats.insert(crate::config::format_key(InfoType::Ip.label(), "1 0 0 : 1"));
        o.enabled.insert(InfoType::IpDeep);
        let j = Job { input: "C:\\a\\애플".into(), output: None, opts: o.clone() };
        let text = encode_job(&j);
        let lines: Vec<String> = text.lines().take_while(|l| *l != "END").map(String::from).collect();
        let d = decode_job(&lines).unwrap();
        assert_eq!(d.input, j.input);
        assert_eq!(d.output, None);
        assert_eq!(d.opts.enabled, o.enabled);
        assert_eq!(d.opts.threshold, 0.8);
        assert!(!d.opts.ml_context);
        assert_eq!(d.opts.exts.enabled, o.exts.enabled);
        assert_eq!(d.opts.custom.len(), 1);
        assert_eq!(d.opts.custom[0].regex.as_str(), "EMP-(\\d{6})\t");
        assert_eq!(d.opts.excluded, o.excluded);
        assert_eq!(d.opts.excluded_formats, o.excluded_formats);
    }

    #[test]
    fn parser_assembles_file_events() {
        let r = FileResult {
            path: "C:\\in\\a.txt".into(),
            out_path: "C:\\out\\a.txt".into(),
            kind: "TEXT".into(),
            size: 12,
            records: vec![Record { original: "010-1234-5678".into(), masked: "***".into(), kind: InfoType::Phone, review: true }],
            modified: true,
            notes: vec!["알림\t1".into()],
        };
        let mut p = Parser::default();
        let evs: Vec<Event> = encode_file(1, 2, &r).lines().filter_map(|l| p.feed(l)).collect();
        assert_eq!(evs.len(), 1);
        let Event::File { done, total, entry } = &evs[0] else { panic!() };
        assert_eq!((*done, *total, entry.size, entry.status), (1, 2, 12, Status::Masked));
        assert_eq!(entry.notes, vec!["알림\t1".to_string()]);
        assert_eq!(entry.records[0].label, "핸드폰번호(검토필요)");
        assert!(entry.records[0].review);
    }
}

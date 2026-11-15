//! 명령줄(CLI) 실행 — 콘솔에서 인자와 함께 실행했을 때 사용.

use crate::common::{mask_text, InfoType};
use crate::config::Options;
use crate::pipeline;
use crate::rules::{comments::lang_from_name, Detector};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
사용법: deid                      (인자 없이 실행하면 GUI)
        deid <검사할 폴더> [옵션]
        deid --text \"검사할 문자열\" [--lang 파일명.확장자]

옵션:
  --out <폴더>            출력 폴더 (기본: <폴더>_검사완료)
  --threshold <0~1>       맥락 모델 채택 확률 (기본 0.7)
  --pattern <라벨=정규식> 사용자 정의 패턴 추가 (여러 번 가능, 그룹1이 있으면 그룹1만 치환)
  --pattern-file <파일>   패턴 파일 (줄마다 라벨=정규식)
  --disable <키,..>       끌 타입
  --only <키,..>          켤 타입만 지정
  --min-run <N>           바이너리 문자열 최소 길이 (기본 6)
  --scan-exe              실행파일도 검사
  --max-size <MB>         이 크기 초과 파일은 미검사 복사 (기본 512)
  --no-ocr                그림 속 글자 OCR 검사 끄기
  --ocr-scales <a,b,..>   OCR 확대 배율 (기본 3,2)
  --ocr-max-mp <MP>       OCR 확대 후 화소 수 상한, 백만 화소 (기본 16, 클수록 정확·느림)
  --no-ocr-enhance        OCR 전 흑백 이진화 끄기 (기본 켬)
  --blur <배수>           흐림 강도 (기본 1.0)
  --keep-names            파일/폴더 이름 비식별화 끄기
  --no-ml                 XGBoost 맥락 탐지(기호가 바뀐 번호) 끄기
  --exclude-ext <a,b,..>  검사하지 않을 확장자 (위장 파일은 그래도 검사)
  --known-ext-only        목록에 없는 확장자는 검사하지 않음
  -q, --quiet             파일별 출력 생략

타입 키: phone card email rrn frn ip mac password cookie session comment";

struct Args {
    input: Option<PathBuf>,
    out: Option<PathBuf>,
    text: Option<String>,
    lang: String,
    quiet: bool,
    opts: Options,
}

fn parse(argv: Vec<String>) -> Result<Args, String> {
    let mut a = Args { input: None, out: None, text: None, lang: "a.txt".into(), quiet: false, opts: Options::default() };
    let mut it = argv.into_iter();
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} 값이 필요합니다"));
        match arg.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "--out" => a.out = Some(val("--out")?.into()),
            "--text" => a.text = Some(val("--text")?),
            "--lang" => a.lang = val("--lang")?,
            "--threshold" => {
                a.opts.threshold = val("--threshold")?.parse().map_err(|_| "threshold 는 0~1 실수".to_string())?
            }
            "--pattern" => a.opts.add_custom(&val("--pattern")?)?,
            "--pattern-file" => a.opts.load_pattern_file(std::path::Path::new(&val("--pattern-file")?))?,
            "--disable" => a.opts.disable(&val("--disable")?)?,
            "--only" => a.opts.only(&val("--only")?)?,
            "--min-run" => a.opts.min_binary_run = val("--min-run")?.parse().map_err(|_| "min-run 은 정수")?,
            "--scan-exe" => a.opts.scan_executables = true,
            "--max-size" => {
                let mb: u64 = val("--max-size")?.parse().map_err(|_| "max-size 는 정수(MB)")?;
                a.opts.max_file_size = mb * 1024 * 1024;
            }
            "--no-ocr" => a.opts.ocr = false,
            "--ocr-scales" => {
                a.opts.ocr_scales = val("--ocr-scales")?
                    .split(',')
                    .map(|x| x.trim().parse::<f32>().map_err(|_| "ocr-scales 는 실수 목록".to_string()))
                    .collect::<Result<_, _>>()?
            }
            "--ocr-max-mp" => {
                let mp: f64 = val("--ocr-max-mp")?.parse().map_err(|_| "ocr-max-mp 는 실수")?;
                a.opts.ocr_max_pixels = (mp * 1_000_000.0) as u64;
            }
            "--no-ocr-enhance" => a.opts.ocr_enhance = false,
            "--blur" => a.opts.blur_strength = val("--blur")?.parse().map_err(|_| "blur 는 실수")?,
            "--keep-names" => a.opts.rename_files = false,
            "--no-ml" => a.opts.ml_context = false,
            "--exclude-ext" => {
                for e in val("--exclude-ext")?.split(',') {
                    a.opts.exts.enabled.remove(&e.trim().trim_start_matches('.').to_lowercase());
                }
            }
            "--known-ext-only" => a.opts.exts.scan_unknown = false,
            "-q" | "--quiet" => a.quiet = true,
            s if s.starts_with('-') => return Err(format!("알 수 없는 옵션: {s}")),
            s => a.input = Some(s.into()),
        }
    }
    Ok(a)
}

pub fn main(argv: Vec<String>) -> ExitCode {
    let args = match parse(argv) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("오류: {e}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let det = Detector::new(args.opts);

    // 문자열 단독 검사 (규칙 디버깅용)
    if let Some(t) = args.text {
        let t = t.replace("\\n", "\n");
        let hits = det.scan(&t, lang_from_name(&args.lang, &t));
        let (masked, recs) = mask_text(&t, &hits);
        for (h, r) in hits.iter().zip(&recs) {
            println!("[{}] p={:.2} {} -> {}", r.label(), h.score, r.original, r.masked);
        }
        println!("결과: {masked}");
        return ExitCode::SUCCESS;
    }

    let Some(input) = args.input else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let started = std::time::Instant::now();
    let summary = match pipeline::run(&input, args.out.as_deref(), &det) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("오류: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut by_type: BTreeMap<InfoType, usize> = BTreeMap::new();
    let mut review = 0usize;
    for n in &summary.renamed {
        for r in &n.records {
            *by_type.entry(r.kind.clone()).or_default() += 1;
            review += r.review as usize;
        }
        if !args.quiet {
            println!("[이름변경] {} 건  {}", n.records.len(), n.path.display());
        }
    }
    for f in &summary.files {
        for r in &f.records {
            *by_type.entry(r.kind.clone()).or_default() += 1;
            review += r.review as usize;
        }
        if !args.quiet {
            let mark = if f.modified { "치환" } else { "그대로" };
            println!("[{mark}] {:<22} 검출 {:>4}  {}", f.kind, f.records.len(), f.path.display());
            for n in &f.notes {
                println!("        - {n}");
            }
        }
    }
    println!("\n── 요약 ─────────────────────────");
    println!("파일 {}개 / 치환된 파일 {}개 / {:.2}초", summary.files.len(), summary.files.iter().filter(|f| f.modified).count(), started.elapsed().as_secs_f32());
    for (k, n) in &by_type {
        println!("  {:<14} {n}", k.label());
    }
    if review > 0 {
        println!("  ※ 검토필요 {review}건 — 애매하여 일단 치환함. 보고서의 \"(검토필요)\" 행을 사람이 확인하세요.");
    }
    println!("출력 폴더: {}", summary.output_dir.display());
    println!("보고서  : {}", summary.report.display());
    ExitCode::SUCCESS
}

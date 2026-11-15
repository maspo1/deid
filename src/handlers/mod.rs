//! 파일 형식별 처리기. `process` 가 매직바이트로 형식을 판별해 알맞은 처리기로 보낸다.
//! zip 내부 항목도 같은 `process` 를 재귀 호출하므로 문서 안의 이미지 등도 처리된다.

pub mod binary;
pub mod image;
pub mod pdf;
pub mod raster;
pub mod text;
pub mod zip;

use crate::common::Record;
use crate::filetype::{detect, FileKind};
use crate::rules::Detector;

/// 처리 결과
#[derive(Default, Debug)]
pub struct Outcome {
    /// 변경된 내용 (None = 원본 그대로)
    pub data: Option<Vec<u8>>,
    pub records: Vec<Record>,
    /// 사용자에게 알릴 참고사항 (미지원, 오류 등)
    pub notes: Vec<String>,
    pub kind: String,
}

pub struct Ctx<'a> {
    pub det: &'a Detector,
    pub depth: u32,
}

impl<'a> Ctx<'a> {
    pub fn new(det: &'a Detector) -> Self {
        Ctx { det, depth: 0 }
    }
    pub fn deeper(&self) -> Ctx<'a> {
        Ctx { det: self.det, depth: self.depth + 1 }
    }
}

/// `name` 은 확장자로 주석 문법을 고르는 데만 쓰인다.
pub fn process(name: &str, bytes: &[u8], ctx: &Ctx) -> Outcome {
    let kind = detect(bytes);
    let mut out = match &kind {
        FileKind::Text(_) => text::process(name, bytes, ctx),
        FileKind::Zip(_) => zip::process(bytes, ctx),
        FileKind::Png => image::process_png(bytes, ctx),
        FileKind::Jpeg => image::process_jpeg(bytes, ctx),
        FileKind::Bmp => image::process_bmp(bytes, ctx),
        FileKind::Pdf => pdf::process(bytes, ctx),
        FileKind::Skip(k) => Outcome { notes: vec![format!("{k}: 검사 대상 아님 - 원본 복사")], ..Default::default() },
        FileKind::Binary(_) => binary::process(bytes, ctx),
        FileKind::Executable(k) => {
            if ctx.det.opts.scan_executables {
                binary::process(bytes, ctx)
            } else {
                Outcome { notes: vec![format!("{k}: 실행파일은 수정하지 않고 복사")], ..Default::default() }
            }
        }
        FileKind::Archive(k) => Outcome { notes: vec![format!("{k}: 압축 해제 미지원 - 원본 복사(미검사)")], ..Default::default() },
    };
    out.kind = kind.label();
    out
}

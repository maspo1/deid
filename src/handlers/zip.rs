//! ZIP 컨테이너 처리 (docx, xlsx, pptx, hwpx, odt, jar, zip).
//! 항목 순서·압축방식·시각을 보존하며 다시 묶는다 (hwpx/odt 의 mimetype 무압축 첫 항목 규칙 유지).
//! 각 항목은 `handlers::process` 로 재귀 처리된다.
//!
//! 문서 안에서 정보가 하나라도 발견되면, 문서 첫 화면을 축소 렌더링한 미리보기 썸네일
//! (`docProps/thumbnail.*`, hwpx `Preview/PrvImage.*`, ODF `Thumbnails/*`)은 전체를 흐림 처리한다.
//! 썸네일은 너무 작아 OCR 로 읽히지 않지만 원본 정보가 축소된 채 남아 있기 때문이다.

use super::{process as process_entry, raster, Ctx, Outcome};
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub fn process(bytes: &[u8], ctx: &Ctx) -> Outcome {
    if ctx.depth >= ctx.det.opts.max_depth {
        return Outcome { notes: vec!["중첩 깊이 초과 - 내부 미검사".into()], ..Default::default() };
    }
    match rebuild(bytes, ctx) {
        Ok(o) => o,
        Err(e) => Outcome { notes: vec![format!("ZIP 처리 실패(원본 복사): {e}")], ..Default::default() },
    }
}

pub fn is_thumbnail(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("docprops/thumbnail.") || n.starts_with("preview/prvimage.") || n.starts_with("thumbnails/")
}

struct Entry {
    name: String,
    opts: SimpleFileOptions,
    is_dir: bool,
    original: Vec<u8>,
    new: Option<Vec<u8>>,
}

fn rebuild(bytes: &[u8], ctx: &Ctx) -> Result<Outcome, Box<dyn std::error::Error>> {
    let mut ar = ZipArchive::new(Cursor::new(bytes))?;
    let mut result = Outcome::default();
    let sub = ctx.deeper();
    let mut entries = Vec::with_capacity(ar.len());

    // 1) 항목별 처리
    for i in 0..ar.len() {
        let mut f = ar.by_index(i)?;
        let name = f.name().to_string();
        let method = match f.compression() {
            CompressionMethod::Stored => CompressionMethod::Stored,
            _ => CompressionMethod::Deflated,
        };
        let mut opts = SimpleFileOptions::default().compression_method(method).large_file(f.size() >= 0xFFFF_FFFF);
        if let Some(t) = f.last_modified() {
            opts = opts.last_modified_time(t);
        }
        if let Some(mode) = f.unix_mode() {
            opts = opts.unix_permissions(mode);
        }
        if f.is_dir() {
            entries.push(Entry { name, opts, is_dir: true, original: vec![], new: None });
            continue;
        }
        let mut buf = Vec::with_capacity(f.size() as usize);
        f.read_to_end(&mut buf)?;
        drop(f);

        let o = process_entry(&name, &buf, &sub);
        result.records.extend(o.records);
        result.notes.extend(o.notes.into_iter().map(|n| format!("{name}: {n}")));
        entries.push(Entry { name, opts, is_dir: false, original: buf, new: o.data });
    }

    // 2) 정보가 있었다면 미리보기 썸네일 전체 흐림
    if !result.records.is_empty() {
        for e in entries.iter_mut().filter(|e| !e.is_dir && is_thumbnail(&e.name)) {
            let src = e.new.as_deref().unwrap_or(&e.original);
            if let Some(b) = raster::blur_whole_bytes(src, ctx.det.opts.blur_strength) {
                e.new = Some(b);
                result.notes.push(format!("{}: 미리보기 썸네일 전체 흐림처리", e.name));
            }
        }
    }

    // 3) 변경이 없으면 원본 그대로
    if entries.iter().all(|e| e.new.is_none()) {
        return Ok(result);
    }
    let mut w = ZipWriter::new(Cursor::new(Vec::with_capacity(bytes.len())));
    for e in entries {
        if e.is_dir {
            w.add_directory(e.name, e.opts)?;
        } else {
            w.start_file(e.name, e.opts)?;
            w.write_all(e.new.as_deref().unwrap_or(&e.original))?;
        }
    }
    result.data = Some(w.finish()?.into_inner());
    Ok(result)
}

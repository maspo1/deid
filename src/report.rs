//! CSV 보고서 (Excel 호환 UTF-8 BOM)
//! 열: 검출된정보, 치환정보, 정보타입, 파일경로

use std::io::Write;
use std::path::Path;

pub struct ReportRow {
    pub original: String,
    pub masked: String,
    pub kind: String,
    pub path: String,
}

fn esc(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn write_csv(path: &Path, rows: &[ReportRow]) -> std::io::Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(&[0xEF, 0xBB, 0xBF])?;
    writeln!(f, "검출된정보,치환정보,정보타입,파일경로")?;
    for r in rows {
        writeln!(f, "{},{},{},{}", esc(&r.original), esc(&r.masked), esc(&r.kind), esc(&r.path))?;
    }
    f.flush()
}

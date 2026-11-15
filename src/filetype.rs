//! 매직바이트 기반 파일 형식 판별.
//! 확장자는 신뢰하지 않으며, 내용으로 실제 형식을 결정한다.

use crate::common::{decode_text, find_bytes, TextEncoding};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileKind {
    Text(TextEncoding),
    /// ZIP 컨테이너 (세부 형식 이름: DOCX/XLSX/PPTX/HWPX/ODF/JAR/ZIP)
    Zip(&'static str),
    Png,
    Jpeg,
    Bmp,
    Pdf,
    /// 검사 대상이 아닌 형식 (PSD, GIF, TIFF) — 원본 그대로 복사
    Skip(&'static str),
    /// 처리 방식이 같은 기타 바이너리 (OLE(doc/xls), SQLite, 미디어 ...)
    Binary(&'static str),
    /// 수정하면 안 되는 형식 (실행파일 등)
    Executable(&'static str),
    /// 내부 해제 미지원 압축 형식
    Archive(&'static str),
}

impl FileKind {
    pub fn label(&self) -> String {
        match self {
            FileKind::Text(e) => format!("TEXT({})", e.label()),
            FileKind::Zip(s) | FileKind::Binary(s) | FileKind::Executable(s) | FileKind::Archive(s) | FileKind::Skip(s) => {
                s.to_string()
            }
            FileKind::Pdf => "PDF".into(),
            FileKind::Png => "PNG".into(),
            FileKind::Jpeg => "JPEG".into(),
            FileKind::Bmp => "BMP".into(),
        }
    }
}

fn zip_flavor(b: &[u8]) -> &'static str {
    let has = |n: &[u8]| find_bytes(b, n).is_some();
    if has(b"application/hwp+zip") || has(b"Contents/section0.xml") {
        "HWPX"
    } else if has(b"word/document.xml") {
        "DOCX"
    } else if has(b"xl/workbook.xml") {
        "XLSX"
    } else if has(b"ppt/presentation.xml") {
        "PPTX"
    } else if has(b"application/vnd.oasis.opendocument") {
        "ODF"
    } else if has(b"META-INF/MANIFEST.MF") {
        "JAR"
    } else {
        "ZIP"
    }
}

pub fn detect(b: &[u8]) -> FileKind {
    let starts = |m: &[u8]| b.starts_with(m);

    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") {
        return FileKind::Zip(zip_flavor(b));
    }
    if starts(b"\x89PNG\r\n\x1a\n") {
        return FileKind::Png;
    }
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return FileKind::Jpeg;
    }
    if starts(b"MZ") {
        return FileKind::Executable("EXE/DLL");
    }
    if starts(b"\x7FELF") {
        return FileKind::Executable("ELF");
    }
    if starts(&[0xCA, 0xFE, 0xBA, 0xBE]) || starts(&[0xCF, 0xFA, 0xED, 0xFE]) || starts(&[0xCE, 0xFA, 0xED, 0xFE]) {
        return FileKind::Executable("MACH-O/CLASS");
    }
    if starts(&[0x1F, 0x8B]) {
        return FileKind::Archive("GZIP");
    }
    if starts(b"7z\xBC\xAF\x27\x1C") {
        return FileKind::Archive("7Z");
    }
    if starts(b"Rar!\x1A\x07") {
        return FileKind::Archive("RAR");
    }
    if starts(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        return FileKind::Archive("XZ");
    }
    if starts(b"BZh") && b.len() > 4 && b[3].is_ascii_digit() {
        return FileKind::Archive("BZIP2");
    }
    if starts(&[0x28, 0xB5, 0x2F, 0xFD]) {
        return FileKind::Archive("ZSTD");
    }
    if starts(b"MSCF") {
        return FileKind::Archive("CAB");
    }
    if starts(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return FileKind::Binary("OLE(DOC/XLS/PPT)");
    }
    // 앞에 쓰레기 바이트가 붙은 PDF 도 허용 (명세상 1KB 이내)
    if find_bytes(&b[..b.len().min(1024)], b"%PDF-").is_some() {
        return FileKind::Pdf;
    }
    if starts(b"8BPS") {
        return FileKind::Skip("PSD");
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return FileKind::Skip("GIF");
    }
    if starts(b"BM") && b.len() > 26 && u32::from_le_bytes([b[2], b[3], b[4], b[5]]) as usize == b.len() {
        return FileKind::Bmp;
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        return FileKind::Skip("TIFF");
    }
    if starts(b"RIFF") {
        return FileKind::Binary("RIFF(WEBP/WAV/AVI)");
    }
    if starts(b"SQLite format 3\0") {
        return FileKind::Binary("SQLITE");
    }
    if b.len() > 8 && &b[4..8] == b"ftyp" {
        return FileKind::Binary("MP4/HEIC");
    }
    if starts(b"ID3") || starts(b"fLaC") || starts(b"OggS") {
        return FileKind::Binary("AUDIO");
    }
    if let Some((_, enc)) = decode_text(b) {
        return FileKind::Text(enc);
    }
    FileKind::Binary("BINARY")
}

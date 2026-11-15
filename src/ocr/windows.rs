//! Windows 10/11 내장 OCR (`Windows.Media.Ocr`, WinRT).

use super::{OcrLine, OcrWord};
use image::RgbaImage;
use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

fn err(e: windows::core::Error) -> String {
    format!("OCR 오류: {}", e.message())
}

fn init() {
    // 이미 초기화된 스레드면 S_FALSE/RPC_E_CHANGED_MODE — 무시해도 됨
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
}

fn engine() -> Result<OcrEngine, String> {
    init();
    if let Ok(e) = OcrEngine::TryCreateFromUserProfileLanguages() {
        return Ok(e);
    }
    for tag in ["ko", "en-US"] {
        if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(tag)) {
            if let Ok(e) = OcrEngine::TryCreateFromLanguage(&lang) {
                return Ok(e);
            }
        }
    }
    Err("사용 가능한 OCR 언어가 없습니다 (Windows 설정 > 언어 > 한국어 OCR 설치 필요)".into())
}

pub fn max_dimension() -> Result<u32, String> {
    init();
    OcrEngine::MaxImageDimension().map_err(err)
}

pub fn recognize(img: &RgbaImage) -> Result<Vec<OcrLine>, String> {
    let engine = engine()?;
    let mut bgra = img.as_raw().clone();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let writer = DataWriter::new().map_err(err)?;
    writer.WriteBytes(&bgra).map_err(err)?;
    let buf = writer.DetachBuffer().map_err(err)?;
    let bmp = SoftwareBitmap::CreateCopyFromBuffer(&buf, BitmapPixelFormat::Bgra8, img.width() as i32, img.height() as i32)
        .map_err(err)?;
    let res = engine.RecognizeAsync(&bmp).map_err(err)?.get().map_err(err)?;
    let mut out = Vec::new();
    for line in res.Lines().map_err(err)? {
        let mut l = OcrLine::default();
        for w in line.Words().map_err(err)? {
            let r = w.BoundingRect().map_err(err)?;
            l.words.push(OcrWord { text: w.Text().map_err(err)?.to_string_lossy(), x: r.X, y: r.Y, w: r.Width, h: r.Height });
        }
        out.push(l);
    }
    Ok(out)
}

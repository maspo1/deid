//! macOS 내장 OCR — Vision 프레임워크 `VNRecognizeTextRequest` (macOS 10.15+, 한국어 인식은 macOS 13+).
//!
//! Vision 은 줄 단위 결과를 주므로, 줄을 공백으로 나눈 단어마다 `boundingBoxForRange` 로 좌표를 얻는다.
//! Vision 좌표는 0~1 정규화 + 왼쪽 아래 원점 → 픽셀 + 왼쪽 위 원점으로 바꿔 Windows 와 같은 형식으로 돌려준다.

use super::{OcrLine, OcrWord};
use image::RgbaImage;
use objc2::rc::autoreleasepool;
use objc2::AllocAnyThread;
use objc2_foundation::{NSArray, NSData, NSDictionary, NSRange, NSString};
use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel};

/// Vision 은 고정 한계가 없으므로 메모리를 고려한 값으로 제한
pub fn max_dimension() -> Result<u32, String> {
    Ok(8192)
}

fn png_bytes(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| format!("OCR 이미지 변환 실패: {e}"))?;
    Ok(out.into_inner())
}

/// 단어(공백 구분)별 (UTF-16 시작, UTF-16 길이, 문자열)
fn words_utf16(line: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let (mut pos, mut start, mut cur) = (0usize, 0usize, String::new());
    for ch in line.chars() {
        if ch.is_whitespace() {
            if !cur.is_empty() {
                out.push((start, pos - start, std::mem::take(&mut cur)));
            }
            pos += ch.len_utf16();
            start = pos;
        } else {
            cur.push(ch);
            pos += ch.len_utf16();
        }
    }
    if !cur.is_empty() {
        out.push((start, pos - start, cur));
    }
    out
}

pub fn recognize(img: &RgbaImage) -> Result<Vec<OcrLine>, String> {
    let png = png_bytes(img)?;
    let (iw, ih) = (img.width() as f64, img.height() as f64);
    autoreleasepool(|_| unsafe {
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setUsesLanguageCorrection(false); // 번호·주소는 사전 교정이 오히려 해롭다
        let langs = NSArray::from_retained_slice(&[NSString::from_str("ko-KR"), NSString::from_str("en-US")]);
        request.setRecognitionLanguages(&langs);

        let data = NSData::with_bytes(&png);
        let handler = VNImageRequestHandler::initWithData_options(VNImageRequestHandler::alloc(), &data, &NSDictionary::new());
        let req: &VNRequest = &request;
        handler
            .performRequests_error(&NSArray::from_slice(&[req]))
            .map_err(|e| format!("OCR 오류: {}", e.localizedDescription()))?;

        let mut out = Vec::new();
        let Some(results) = request.results() else { return Ok(out) };
        for obs in results.iter() {
            let cands = obs.topCandidates(1);
            let Some(best) = cands.firstObject() else { continue };
            let text = best.string().to_string();
            let mut line = OcrLine::default();
            for (start, len, word) in words_utf16(&text) {
                let Ok(r) = best.boundingBoxForRange_error(NSRange::new(start, len)) else { continue };
                let b = r.boundingBox();
                line.words.push(OcrWord {
                    text: word,
                    x: (b.origin.x * iw) as f32,
                    y: ((1.0 - b.origin.y - b.size.height) * ih) as f32,
                    w: (b.size.width * iw) as f32,
                    h: (b.size.height * ih) as f32,
                });
            }
            if !line.words.is_empty() {
                out.push(line);
            }
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn utf16_word_ranges() {
        let w = super::words_utf16("전화 010-1234-5678  끝");
        assert_eq!(w.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(), vec![(0, 2), (3, 13), (18, 1)]);
    }
}

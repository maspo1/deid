//! 일반 바이너리 처리 (PDF, OLE 문서, PSD, 기타).
//! 내부의 ASCII / UTF-16LE 문자열을 추출해 검사하고, **같은 길이**로 제자리 치환한다.
//! 길이가 변하지 않으므로 파일 구조(오프셋)가 깨지지 않는다.

use super::{Ctx, Outcome};
use crate::common::{char_mask, records, Record};
use crate::rules::{Detector, Lang};

#[inline]
fn ascii_text_byte(b: u8) -> bool {
    (0x20..0x7F).contains(&b) || b == b'\t' || b == b'\n' || b == b'\r'
}

#[inline]
fn utf16_text_unit(u: u16) -> bool {
    (0x20..0x7F).contains(&u) || u == 9 || u == 10 || u == 13 || (0xAC00..=0xD7A3).contains(&u) || (0x3131..=0x318E).contains(&u)
}

/// 버퍼 내 문자열을 찾아 제자리 마스킹, 처리 내역 반환
pub fn mask_in_place(buf: &mut [u8], det: &Detector) -> Vec<Record> {
    let min = det.opts.min_binary_run.max(4);
    let mut recs = Vec::new();
    let n = buf.len();

    // 1) ASCII 문자열
    let mut i = 0;
    while i < n {
        if !ascii_text_byte(buf[i]) {
            i += 1;
            continue;
        }
        let s = i;
        while i < n && ascii_text_byte(buf[i]) {
            i += 1;
        }
        if i - s >= min {
            let text = std::str::from_utf8(&buf[s..i]).unwrap().to_string();
            let hits = det.scan(&text, Lang::None);
            if !hits.is_empty() {
                let mask = char_mask(&text, &hits);
                for (k, &m) in mask.iter().enumerate() {
                    if m {
                        buf[s + k] = b'*';
                    }
                }
                recs.extend(records(&text, &hits));
            }
        }
    }

    // 2) UTF-16LE 문자열 (짝수/홀수 정렬 모두)
    for align in 0..2 {
        let mut i = align;
        while i + 1 < n {
            let u = u16::from_le_bytes([buf[i], buf[i + 1]]);
            if !utf16_text_unit(u) {
                i += 2;
                continue;
            }
            let s = i;
            let mut units = Vec::new();
            while i + 1 < n {
                let u = u16::from_le_bytes([buf[i], buf[i + 1]]);
                if !utf16_text_unit(u) {
                    break;
                }
                units.push(u);
                i += 2;
            }
            // ASCII 만으로 된 짧은 우연한 조합 배제: 최소 길이 + ASCII 문자 포함
            if units.len() >= min && units.iter().any(|&u| u < 0x80) {
                let text = String::from_utf16(&units).unwrap();
                let hits = det.scan(&text, Lang::None);
                if !hits.is_empty() {
                    let mask = char_mask(&text, &hits);
                    for (k, &m) in mask.iter().enumerate() {
                        if m {
                            buf[s + 2 * k] = b'*';
                            buf[s + 2 * k + 1] = 0;
                        }
                    }
                    recs.extend(records(&text, &hits));
                }
            }
        }
    }
    recs
}

pub fn process(bytes: &[u8], ctx: &Ctx) -> Outcome {
    let mut buf = bytes.to_vec();
    let records = mask_in_place(&mut buf, ctx.det);
    if records.is_empty() {
        return Outcome::default();
    }
    Outcome { data: Some(buf), records, ..Default::default() }
}

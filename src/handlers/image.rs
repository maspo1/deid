//! 이미지 처리 — 두 단계
//! 1. 메타데이터(텍스트/EXIF/XMP/주석) 검사·치환
//!    * PNG : tEXt / iTXt / zTXt / eXIf 청크 (치환 후 CRC 재계산)
//!    * JPEG: APP0~APP15(EXIF, XMP 등) / COM 세그먼트 (같은 길이 제자리 치환)
//! 2. 그림 속 글자: OCR → 정보 영역 흐림 처리 (`raster`)

use super::binary::mask_in_place;
use super::{raster, Ctx, Outcome};
use crate::common::Record;
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use std::io::{Read, Write};

fn inflate(d: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    ZlibDecoder::new(d).read_to_end(&mut out).ok()?;
    Some(out)
}

fn deflate(d: &[u8]) -> Vec<u8> {
    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    e.write_all(d).unwrap();
    e.finish().unwrap()
}

/// 압축된 텍스트 블록 처리 → 변경 시 재압축 결과
fn mask_compressed(comp: &[u8], ctx: &Ctx, recs: &mut Vec<Record>) -> Option<Vec<u8>> {
    let mut plain = inflate(comp)?;
    let r = mask_in_place(&mut plain, ctx.det);
    if r.is_empty() {
        return None;
    }
    recs.extend(r);
    Some(deflate(&plain))
}

/// PNG 텍스트 청크 하나 처리 → 변경된 청크 데이터
fn png_chunk(ty: &[u8], data: &[u8], ctx: &Ctx, recs: &mut Vec<Record>) -> Option<Vec<u8>> {
    match ty {
        b"tEXt" | b"eXIf" => {
            let mut d = data.to_vec();
            let r = mask_in_place(&mut d, ctx.det);
            (!r.is_empty()).then(|| {
                recs.extend(r);
                d
            })
        }
        b"zTXt" => {
            // keyword \0 method(1) compressed
            let k = data.iter().position(|&b| b == 0)?;
            let body = mask_compressed(data.get(k + 2..)?, ctx, recs)?;
            let mut d = data[..k + 2].to_vec();
            d.extend(body);
            Some(d)
        }
        b"iTXt" => {
            // keyword \0 flag(1) method(1) lang \0 translated \0 text
            let k = data.iter().position(|&b| b == 0)?;
            let flag = *data.get(k + 1)?;
            let l = k + 3 + data.get(k + 3..)?.iter().position(|&b| b == 0)?;
            let t = l + 1 + data.get(l + 1..)?.iter().position(|&b| b == 0)?;
            let head = &data[..=t];
            let text = &data[t + 1..];
            let new_text = if flag == 1 {
                mask_compressed(text, ctx, recs)?
            } else {
                let mut d = text.to_vec();
                let r = mask_in_place(&mut d, ctx.det);
                if r.is_empty() {
                    return None;
                }
                recs.extend(r);
                d
            };
            let mut d = head.to_vec();
            d.extend(new_text);
            Some(d)
        }
        _ => None,
    }
}

fn png_metadata(bytes: &[u8], ctx: &Ctx) -> Outcome {
    let n = bytes.len();
    let mut out = bytes[..8.min(n)].to_vec();
    let mut recs = Vec::new();
    let mut changed = false;
    let mut pos = 8;
    while pos + 12 <= n {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        let end = pos + 12 + len;
        if end > n {
            break;
        }
        let ty = &bytes[pos + 4..pos + 8];
        let data = &bytes[pos + 8..pos + 8 + len];
        match png_chunk(ty, data, ctx, &mut recs) {
            Some(d) => {
                let mut h = crc32fast::Hasher::new();
                h.update(ty);
                h.update(&d);
                out.extend((d.len() as u32).to_be_bytes());
                out.extend(ty);
                out.extend(&d);
                out.extend(h.finalize().to_be_bytes());
                changed = true;
            }
            None => out.extend(&bytes[pos..end]),
        }
        pos = end;
        if ty == b"IEND" {
            break;
        }
    }
    out.extend(&bytes[pos.min(n)..]); // IEND 뒤 덧붙은 데이터 보존
    if !changed {
        return Outcome { records: recs, ..Default::default() };
    }
    Outcome { data: Some(out), records: recs, ..Default::default() }
}

fn jpeg_metadata(bytes: &[u8], ctx: &Ctx) -> Outcome {
    let mut d = bytes.to_vec();
    let n = d.len();
    let mut recs = Vec::new();
    let mut pos = 2;
    while pos + 4 <= n {
        if d[pos] != 0xFF {
            break;
        }
        let m = d[pos + 1];
        if m == 0xFF {
            pos += 1;
            continue;
        }
        if m == 0x01 || (0xD0..=0xD8).contains(&m) {
            pos += 2;
            continue;
        }
        if m == 0xDA || m == 0xD9 {
            break; // 영상 데이터 시작 → 이후는 픽셀
        }
        let len = u16::from_be_bytes([d[pos + 2], d[pos + 3]]) as usize;
        if len < 2 || pos + 2 + len > n {
            break;
        }
        if (0xE0..=0xEF).contains(&m) || m == 0xFE {
            recs.extend(mask_in_place(&mut d[pos + 4..pos + 2 + len], ctx.det));
        }
        pos += 2 + len;
    }
    if recs.is_empty() {
        return Outcome::default();
    }
    Outcome { data: Some(d), records: recs, ..Default::default() }
}

/// 메타데이터 결과와 OCR 결과 합치기.
/// OCR 로 흐림 처리하면 재인코딩되며 메타데이터 청크는 제거되므로 픽셀 결과를 우선한다.
fn combine(meta: Outcome, pixels: Outcome) -> Outcome {
    let mut out = meta;
    out.records.extend(pixels.records);
    out.notes.extend(pixels.notes);
    if pixels.data.is_some() {
        out.data = pixels.data;
    }
    out
}

pub fn process_png(bytes: &[u8], ctx: &Ctx) -> Outcome {
    combine(png_metadata(bytes, ctx), raster::process(bytes, image::ImageFormat::Png, ctx))
}

pub fn process_jpeg(bytes: &[u8], ctx: &Ctx) -> Outcome {
    combine(jpeg_metadata(bytes, ctx), raster::process(bytes, image::ImageFormat::Jpeg, ctx))
}

pub fn process_bmp(bytes: &[u8], ctx: &Ctx) -> Outcome {
    let mut o = raster::process(bytes, image::ImageFormat::Bmp, ctx);
    if o.data.is_none() && o.records.is_empty() {
        // OCR 결과가 없으면 일반 바이너리 문자열 검사
        o = super::binary::process(bytes, ctx);
    }
    o
}

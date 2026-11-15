//! 텍스트 파일 처리 (txt, log, json, csv, 소스코드, xml ...). 원래 인코딩을 그대로 유지한다.

use super::{Ctx, Outcome};
use crate::common::{decode_text, encode_text, mask_text};
use crate::rules::comments::lang_from_name;

pub fn process(name: &str, bytes: &[u8], ctx: &Ctx) -> Outcome {
    let Some((text, enc)) = decode_text(bytes) else {
        return super::binary::process(bytes, ctx);
    };
    let lang = lang_from_name(name, &text);
    let hits = ctx.det.scan(&text, lang);
    if hits.is_empty() {
        return Outcome::default();
    }
    let (masked, records) = mask_text(&text, &hits);
    Outcome { data: Some(encode_text(&masked, enc)), records, ..Default::default() }
}

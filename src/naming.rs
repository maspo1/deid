//! 파일·폴더 이름 비식별화.
//! Windows 파일명에는 `*` 를 쓸 수 없으므로 모양이 같은 전각 별표 `＊`(U+FF0A)로 치환한다.
//! 치환 후 같은 폴더에 같은 이름이 생기면 ` (2)`, ` (3)` … 을 붙인다.

use crate::common::{char_mask, mask_str_with, InfoType, Record};
use crate::rules::{Detector, Lang};
use std::collections::HashSet;

pub const NAME_MASK_CHAR: char = '＊';

/// 이름에 정보가 있으면 (새 이름, 처리 내역)
pub fn deidentify(name: &str, det: &Detector) -> Option<(String, Vec<Record>)> {
    if !det.opts.rename_files {
        return None;
    }
    let hits: Vec<_> = det.scan(name, Lang::None).into_iter().filter(|h| h.kind != InfoType::Comment).collect();
    if hits.is_empty() {
        return None;
    }
    let mask = char_mask(name, &hits);
    let new: String = name.chars().zip(mask).map(|(c, m)| if m { NAME_MASK_CHAR } else { c }).collect();
    let recs = hits
        .iter()
        .map(|h| {
            let s = &name[h.start..h.end];
            Record { original: s.to_string(), masked: mask_str_with(s, h.mode, NAME_MASK_CHAR), kind: h.kind.clone(), review: h.review }
        })
        .collect();
    Some((new, recs))
}

/// 폴더 안에서 겹치지 않는 이름 (대소문자 무시)
pub fn unique(name: &str, taken: &mut HashSet<String>) -> String {
    let mut cand = name.to_string();
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let mut n = 2;
    while taken.contains(&cand.to_lowercase()) {
        cand = format!("{stem} ({n}){ext}");
        n += 1;
    }
    taken.insert(cand.to_lowercase());
    cand
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Options;

    #[test]
    fn rename_phone() {
        let det = Detector::new(Options::default());
        let (n, r) = deidentify("010-1200-5678.txt", &det).unwrap();
        assert_eq!(n, "＊＊＊-＊＊＊＊-＊＊＊＊.txt");
        assert_eq!(r[0].original, "010-1200-5678");
        assert!(deidentify("보고서.txt", &det).is_none());
    }

    #[test]
    fn unique_names() {
        let mut t = HashSet::new();
        assert_eq!(unique("a.txt", &mut t), "a.txt");
        assert_eq!(unique("A.txt", &mut t), "A (2).txt");
    }
}

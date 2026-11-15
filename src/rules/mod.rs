//! 탐지 규칙 묶음.
//!
//! 처리 순서 (AGENTS.md)
//!   1단계 정보 매칭(`patterns`, `context` 후보) → 2단계 사전 규칙(`dict`) → 3단계 둘 다 통과 시 치환
//! 맥락 정보(패스워드/쿠키/세션)는 `context` 의 모델 확률 p ≥ 임계값일 때만 채택한다.
//! 기호가 바뀐 번호는 `ml`(XGBoost 방식 트리 부스팅 + 소프트맥스)이 추론하고 `dict` 로 다시 확인한다.

pub mod comments;
pub mod context;
pub mod dict;
pub mod matrix;
pub mod ml;
pub mod ocr_context;
pub mod patterns;

use crate::common::{resolve_overlaps, Hit, InfoType, MaskMode};
use crate::config::Options;
pub use comments::Lang;

pub struct Detector {
    pub opts: Options,
    patterns: patterns::Patterns,
    matrix: matrix::Matrix,
    creds: context::CredentialScanner,
}

impl Detector {
    pub fn new(opts: Options) -> Self {
        Detector { opts, patterns: patterns::Patterns::new(), matrix: matrix::Matrix::new(), creds: context::CredentialScanner::new() }
    }

    /// 텍스트 검사 → 치환할 위치 목록 (시작 위치 순)
    pub fn scan(&self, text: &str, lang: Lang) -> Vec<Hit> {
        let o = &self.opts;
        let mut hits = Vec::new();

        // 전화·주민/외국인·카드·IPv4 — 정답 행렬로 한 번에 훑음 (matrix.rs)
        self.matrix.scan(text, |k| o.is_on(k), &mut hits);
        if o.is_on(&InfoType::Mac) {
            self.patterns.macs(text, &mut hits);
        }
        // 국제 표기(+82)만 정규식 경로 — 없으면 건너뜀
        if o.is_on(&InfoType::Phone) && text.contains("+82") {
            self.patterns.phones(text, &mut hits);
        }
        if o.is_on(&InfoType::Email) {
            self.patterns.emails(text, &mut hits);
        }
        // 숫자 묶음 후보는 한 번만 만들어 기계적 전화번호 규칙과 ML 이 같이 쓴다
        let want_mech = o.is_on(&InfoType::Phone);
        if want_mech || o.ml_context {
            let cands = ml::features::candidates(text);
            if want_mech {
                self.patterns.phones_mechanical_in(text, &cands, &mut hits);
            }
            // 기호가 바뀐 번호 (0109999-1234, 192-168-0-1 …) — 정규식 결과와 겹치면 정규식이 우선
            if o.ml_context {
                ml::scan_candidates(text, cands, o, &mut hits);
            }
        }
        if o.is_on(&InfoType::Password) || o.is_on(&InfoType::Cookie) || o.is_on(&InfoType::Session) {
            self.creds.scan(text, o.threshold, &mut hits);
        }
        for cp in &o.custom {
            for c in cp.regex.captures_iter(text) {
                let m = c.get(1).or_else(|| c.get(0)).unwrap();
                hits.push(Hit::new(m.start(), m.end(), InfoType::Custom(cp.label.clone()), MaskMode::Alnum));
            }
        }
        // 이미 치환된 값(****)처럼 영숫자가 없는 것은 정보가 아님 → 재검사 시 결과 불변(멱등)
        hits.retain(|h| {
            o.is_on(&h.kind)
                && (h.score >= o.threshold || h.review)
                && text[h.start..h.end].chars().any(|c| c.is_alphanumeric())
        });
        let mut hits = resolve_overlaps(hits);

        // 주석은 다른 검출과 겹쳐도 그대로 함께 기록 (주석 안 개인정보도 보고서에 남김)
        if lang.is_code() && o.is_on(&InfoType::Comment) {
            for (s, e) in comments::find_comments(text, lang) {
                hits.push(Hit::new(s, e, InfoType::Comment, MaskMode::NonSpace));
            }
            hits.sort_by_key(|h| h.start);
        }
        // 사용자가 제외한 값 (보고서 레코드의 원본 값과 같은 기준: 앞뒤 공백 제거 후 완전 일치)
        if !o.excluded.is_empty() {
            hits.retain(|h| !o.excluded.contains(text[h.start..h.end].trim()));
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_values_are_not_hit() {
        let text = "담당 010-1234-5678, 부담당 010-9999-0000";
        let mut o = Options::default();
        assert_eq!(Detector::new(o.clone()).scan(text, Lang::None).len(), 2);
        o.excluded.insert("010-1234-5678".into());
        let hits = Detector::new(o).scan(text, Lang::None);
        assert_eq!(hits.len(), 1);
        assert_eq!(&text[hits[0].start..hits[0].end], "010-9999-0000");
    }
}

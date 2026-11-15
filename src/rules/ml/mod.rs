//! 맥락 기반 기계학습 탐지 — 기호가 바뀐(변형된) 전화번호·IP·카드·주민번호.
//!
//! 정규식은 `010-9999-1234` 처럼 정해진 모양만 잡는다. 유출자가 `sed` 등으로 구분자를 바꾸거나 빼면
//! (`0109999-1234`, `192-168-0-1`, `4111_1111_1111_1111`) 정규식을 피할 수 있다.
//! 여기서는 숫자 묶음 덩어리를 후보로 뽑아 **맥락 특징 벡터**를 만들고, XGBoost 방식 트리 부스팅 모델의
//! **소프트맥스 확률**로 정보 종류를 추론한다.
//!
//! AGENTS.md 3단계 원칙
//! 1. 모델 확률 p (클래스 = none/phone/ip/card/rrn)
//! 2. 사전 규칙 — 숫자만 뽑아 기존 규칙 검사 (010 국번, 옥텟 ≤ 255, Luhn, 생년월일·성별)
//! 3. 둘 다 통과 → 치환. p 가 임계값(0.7) 미만이지만 0.5 이상이면 치환 + 검토필요
//!
//! 2단 구조
//! * 1단 `natural.txt` — 자연스러움 모델: IP 사전(사설·공인)으로 만든 실제 IP 분포 vs 사람이 만든 배열
//!   (12-34-56-78, 10-20-30-40 …) 을 구분. 출력 확률이 2단의 특징이 된다.
//! * 2단 `model.txt` — 본 모델: 후보 특징 + 자연스러움 + 사전 조회 + 문서 단위 기계적 치환 흔적 + 다의성
//!
//! * `features`  : 후보 추출 + 특징 벡터 (학습·추론 공용)
//! * `ipdict`    : 사설/특수 대역 사전, 공인 대역 사전 (IANA /8, 국내, 클라우드, 공용 DNS)
//! * `intuition` : 인간 직관 특징 (연속 숫자, 등차, 반복, 대칭) + 1단 학습 데이터
//! * `gbdt`      : XGBoost 알고리즘 (학습, 추론, 모델 직렬화) — 순수 Rust
//! * `synth`     : 2단 학습용 합성 문서
//! * 재학습: `cargo run --release --example train_ml`

pub mod features;
pub mod gbdt;
pub mod intuition;
pub mod ipdict;
pub mod synth;

use crate::common::{Hit, InfoType, MaskMode};
use crate::config::Options;
use crate::rules::dict;
use features::{Candidate, C_CARD, C_IP, C_PHONE, C_RRN};
use std::sync::OnceLock;

/// 이 확률 미만은 버린다 (그 이상 ~ 임계값 미만은 치환 + 검토)
pub const REVIEW_FLOOR: f32 = 0.5;

static MODEL_TEXT: &str = include_str!("model.txt");
static NATURAL_TEXT: &str = include_str!("natural.txt");

/// 2단 본 모델
pub fn model() -> &'static gbdt::Model {
    static M: OnceLock<gbdt::Model> = OnceLock::new();
    M.get_or_init(|| gbdt::Model::from_text(MODEL_TEXT).expect("내장 ML 모델 손상"))
}

/// 1단 자연스러움 모델
pub fn natural_model() -> &'static gbdt::Model {
    static M: OnceLock<gbdt::Model> = OnceLock::new();
    M.get_or_init(|| gbdt::Model::from_text(NATURAL_TEXT).expect("내장 자연스러움 모델 손상"))
}

/// 2단계: 모델이 고른 클래스가 사전 규칙도 통과하는가 → 정보 타입
fn gate(class: usize, c: &Candidate) -> Option<InfoType> {
    let d = c.digits();
    match class {
        C_PHONE => {
            let (a, m, l) = features::mobile_parts(&d)?;
            dict::valid_mobile(a, m, l).then_some(InfoType::Phone)
        }
        C_IP => {
            let ip = features::ip_string(c)?;
            // 점(.)으로만 이어진 IP 는 정규식 단계의 구조 규칙(좌표·OID·검토)이 담당
            if c.seps.iter().all(|s| s == ".") {
                return None;
            }
            let o = features::quad(c)?;
            // 루프백·문서용 예시·멀티캐스트·예약 대역은 사람(장비)을 식별하지 않음
            let usable = !ipdict::info(&o).non_host && !intuition::unusable(&o);
            (dict::valid_ipv4(&ip) && !dict::ipv4_is_trivial(&ip) && usable).then_some(InfoType::Ip)
        }
        C_CARD => dict::valid_card(&d).then_some(InfoType::Card),
        C_RRN if d.len() == 13 => dict::rrn_kind(&d[..6], &d[6..], c.groups.len() > 1),
        _ => None,
    }
}

/// 후보별 판단 결과 (디버깅·테스트용)
pub struct Judged {
    pub start: usize,
    pub end: usize,
    pub probs: Vec<f32>,
    pub kind: Option<InfoType>,
}

pub fn judge(text: &str) -> Vec<Judged> {
    judge_candidates(text, features::candidates(text), false)
}

/// `only_possible`: 어느 클래스로도 사전 규칙(gate)을 통과할 수 없는 후보는 모델을 돌리지 않는다.
/// 결과 타입은 gate(모델이 고른 클래스) 이므로, 통과 가능한 클래스가 없으면 모델이 뭘 고르든 검출이 아니다 → 탐지 결과 동일.
fn judge_candidates(text: &str, cands: Vec<features::Candidate>, only_possible: bool) -> Vec<Judged> {
    let m = model();
    let possible = |c: &Candidate| !only_possible || [C_PHONE, C_IP, C_CARD, C_RRN].iter().any(|&k| gate(k, c).is_some());
    features::doc_features_keep(text, cands, natural_model(), possible)
        .into_iter()
        .map(|(c, x)| {
            let probs = m.predict_proba(&x);
            let (cls, _) = probs.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
            Judged { start: c.start, end: c.end, kind: gate(cls, &c), probs }
        })
        .collect()
}

/// 탐지: 사전 규칙을 통과하고 p ≥ REVIEW_FLOOR 인 후보를 Hit 로 (p < threshold 면 검토필요)
pub fn scan(text: &str, opts: &Options, out: &mut Vec<Hit>) {
    scan_candidates(text, features::candidates(text), opts, out)
}

/// 미리 만든 숫자 묶음 후보로 탐지 (Detector 가 기계적 전화번호 규칙과 후보를 함께 씀)
pub fn scan_candidates(text: &str, cands: Vec<features::Candidate>, opts: &Options, out: &mut Vec<Hit>) {
    if model().trees.is_empty() || natural_model().trees.is_empty() {
        return;
    }
    for j in judge_candidates(text, cands, true) {
        let Some(kind) = j.kind else { continue };
        if !opts.is_on(&kind) {
            continue;
        }
        let cls = match kind {
            InfoType::Phone => C_PHONE,
            InfoType::Ip => C_IP,
            InfoType::Card => C_CARD,
            _ => C_RRN,
        };
        let p = j.probs[cls];
        if p < REVIEW_FLOOR {
            continue;
        }
        let hit = Hit::new(j.start, j.end, kind, MaskMode::Alnum).with_score(p);
        out.push(if p < opts.threshold { hit.needs_review() } else { hit });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// gate 를 통과할 수 없는 후보를 건너뛰어도, 모든 후보를 모델에 넣은 결과(judge)와 탐지가 같다
    #[test]
    fn skipping_impossible_candidates_keeps_hits() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut rnd = |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        let pieces = [
            "010", "9999", "1234", "0109999-1234", "192-168-0-1", "10_20_30_40", "4111", "1111", "-", "_", "/", " ", ".", "881111",
            "1234567", "서버", "연락처 ", "카드 ", "주민 ", "2024", "10", "26", "\n", "ip=", "(010)", "12", "255", "7",
        ];
        let o = Options::default();
        let mut total = 0;
        for _ in 0..3000 {
            let n = 5 + rnd(30);
            let t: String = (0..n).map(|_| pieces[rnd(pieces.len() as u64) as usize]).collect();
            let mut got = Vec::new();
            scan(&t, &o, &mut got);
            // 기준: 모든 후보를 판단한 judge() 로 같은 규칙 적용
            let mut want = Vec::new();
            for j in judge(&t) {
                let Some(kind) = j.kind else { continue };
                let cls = match kind {
                    InfoType::Phone => C_PHONE,
                    InfoType::Ip => C_IP,
                    InfoType::Card => C_CARD,
                    _ => C_RRN,
                };
                if j.probs[cls] >= REVIEW_FLOOR && o.is_on(&kind) {
                    want.push((j.start, j.end, kind, j.probs[cls] < o.threshold));
                }
            }
            let got: Vec<_> = got.into_iter().map(|h| (h.start, h.end, h.kind, h.review)).collect();
            assert_eq!(got, want, "{t:?}");
            total += got.len();
        }
        assert!(total > 300, "검출 표본이 너무 적음: {total}");
    }
}

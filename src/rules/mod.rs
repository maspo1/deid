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
        // 해외핸드폰번호(+국가번호) — + 가 없으면 건너뜀
        if o.is_on(&InfoType::IntlPhone) && text.contains('+') {
            self.patterns.intl_phones(text, &mut hits);
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
            // 점을 한 기호로 일괄 치환한 IP 는 표 검사로 확정 (모델 앞단) — IP주소(심층)을 켰을 때만
            let mut cands = cands;
            if o.ip_deep() {
                let done = ml::substituted_ips(text, &cands, &mut hits);
                cands.retain(|c| !done.contains(&(c.start, c.end)));
            }
            // 그 밖에 기호가 바뀐 번호 (0109999-1234, 192_168.0-1 …) — 정규식 결과와 겹치면 정규식이 우선
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

        // 개인정보·민감정보가 들어 있는 주석만 주석째 치환 (평범한 설명 주석은 그대로 둔다).
        // 주석 검출은 안의 검출과 겹쳐도 함께 기록한다 (보고서에 둘 다 남김).
        if lang.is_code() && o.is_on(&InfoType::Comment) {
            let found: Vec<(usize, usize)> = comments::find_comments(text, lang)
                .into_iter()
                .filter(|&(s, e)| hits.iter().any(|h| h.start >= s && h.end <= e))
                .collect();
            for (s, e) in found {
                hits.push(Hit::new(s, e, InfoType::Comment, MaskMode::NonSpace));
            }
            hits.sort_by_key(|h| h.start);
        }
        // 사용자가 제외한 값 (보고서 레코드의 원본 값과 같은 기준: 앞뒤 공백 제거 후 완전 일치)
        if !o.excluded.is_empty() {
            hits.retain(|h| !o.excluded.contains(text[h.start..h.end].trim()));
        }
        // 사용자가 제외한 형식 (같은 정보타입에서 숫자만 다른 값 모두)
        if !o.excluded_formats.is_empty() {
            hits.retain(|h| !o.excluded_formats.contains(&crate::config::format_key(h.kind.label(), &text[h.start..h.end])));
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_values_are_not_hit() {
        let text = "담당 010-1234-5678, 부담당 010-9999-1111";
        let mut o = Options::default();
        assert_eq!(Detector::new(o.clone()).scan(text, Lang::None).len(), 2);
        o.excluded.insert("010-1234-5678".into());
        let hits = Detector::new(o).scan(text, Lang::None);
        assert_eq!(hits.len(), 1);
        assert_eq!(&text[hits[0].start..hits[0].end], "010-9999-1111");
    }

    fn found_with(o: Options, text: &str, lang: Lang) -> Vec<(String, InfoType, bool)> {
        Detector::new(o).scan(text, lang).iter().map(|h| (text[h.start..h.end].to_string(), h.kind.clone(), h.review)).collect()
    }

    /// 기본 설정 (IP주소는 점 표기만)
    fn found(text: &str, lang: Lang) -> Vec<(String, InfoType, bool)> {
        found_with(Options::default(), text, lang)
    }

    /// IP주소(심층) 켬
    fn found_deep(text: &str, lang: Lang) -> Vec<(String, InfoType, bool)> {
        let mut o = Options::default();
        o.enabled.insert(InfoType::IpDeep);
        found_with(o, text, lang)
    }

    /// 실제 시험 폴더에서 나온 오탐 — 검출되면 안 된다
    #[test]
    fn reported_false_positives() {
        for t in [
            "880922000000",
            "8809220000000",
            "880922-0000000",
            "880922-1111111",
            "880922-9123456",
            "250117,250118,2502",
            "260116,260117,2601",
            "010-1234-0000",
            "01012340000",
            "010-0000-1234",
            // 2차: 실제 문서(PDF 등) 오답
            "00013027665 14",
            "00013027665 7 8",
            "120712, 0.988423",
            "511.05667 -625.17676",
            "66669 -565.33331 608 -584",
            "44610,1903,2491,855278",
            "55117,932,81316,308",
            "6973025 322.1098442",
            "28-35, 97-98",
            "68, 86, 106-109",
            "104-105, 106-109",
            "205-1~205-3",
            "2.3.0.0",
            "129  4  22(21",
            "31.6→35.8",
            "152.1 145 150",
            "0.240.170.17",
            "11.7.8.5.122.90.3.1.7.x",
            "122.90 -> 3.1",
            "image.png",
            "IMG_20240117_120712.png",
            "id=3608-9308-3116-05",
            "0x01037950643x",
            "release-21.29.169.88",
            // 3차: 하나의 규칙으로 복원할 수 없는 숫자 덩어리
            "21.0.ㅇ.88.11.25.1",
            "21.0.88.11.25.1.7",
            "3^4*55-12^9*200-1",
            "206*203_554$51~126.165.8.187$188.3068",
            // 4차: 기호가 섞인 숫자 (비율 표기) · 010 이 아닌 휴대폰 국번
            "1 0 0 : 1",
            "비율 1 0 0 : 1 입니다",
            "011-234-5678",
            "0162345678",
            "019 1234 5678",
        ] {
            assert!(found(t, Lang::None).is_empty(), "{t}: {:?}", found(t, Lang::None));
            assert!(found_deep(t, Lang::None).is_empty(), "심층 {t}: {:?}", found_deep(t, Lang::None));
        }
    }

    /// 레코드 구분자로 이은 묶음은 레코드마다 복원 (프로그램 변형 포함)
    #[test]
    fn recoverable_records() {
        let ips = |t: &str| found_deep(t, Lang::None).into_iter().filter(|(_, k, _)| *k == InfoType::Ip).map(|(v, ..)| v).collect::<Vec<_>>();
        assert_eq!(ips("88.11.25.1,14.20.1.3"), vec!["88.11.25.1", "14.20.1.3"]);
        assert_eq!(ips("88^11^25*1,88&11$25*1"), vec!["88^11^25*1", "88&11$25*1"]);
        assert_eq!(ips("[88^11^25*1][88&11$25*1]"), vec!["88^11^25*1", "88&11$25*1"]);
        assert_eq!(ips("host 192.168.0.1~192.168.0.9"), vec!["192.168.0.1", "192.168.0.9"]);
        assert_eq!(ips("192_168_0_1:8080"), vec!["192_168_0_1"]);
    }

    /// 점을 한 기호로 일괄 치환한 IP 는 맥락이 없어도 확정 (표 검사, 모델 앞단)
    #[test]
    fn uniform_substitution_ips() {
        for t in ["103^234^126^56", "130-170-164-10", "34~127~169~42", "123;136;17;184", "116)221)143)172", "219*142*83*245", "187-0-144-254", "55-_228-_224-_52", "118)46)24)38", "105 -> 174 -> 171 -> 43", "113 - 168 - 253 - 75", "211. 234. 10. 5"] {
            assert_eq!(found_deep(t, Lang::None), vec![(t.to_string(), InfoType::Ip, false)], "{t}");
        }
        assert_eq!(found_deep("ip=104^200^101^188", Lang::None), vec![("104^200^101^188".to_string(), InfoType::Ip, false)]);
        // 숫자표 열 구분자·다른 해석(월·일, 시각)은 규칙으로 확정하지 않는다
        for t in ["10-26-10-31", "12:30:45:10", "10/20/30/40"] {
            assert!(found_deep(t, Lang::None).iter().all(|(_, _, review)| *review), "{t}: {:?}", found_deep(t, Lang::None));
        }
        // 사람이 만든 배열은 치환 + 검토필요
        assert_eq!(found_deep("12^34^56^78", Lang::None), vec![("12^34^56^78".to_string(), InfoType::Ip, true)]);
        // 숫자표 열 구분자(공백 탭 , | /)로 같은 규칙: IP 맥락이면 표 검사로 확정(인공 배열은 검토),
        // 맥락이 없으면 모델이 판단 — 한 규칙 구분자라 낮은 문턱(REGULAR_FLOOR)으로 치환 + 검토필요
        assert_eq!(found_deep("ip 22/222/222/22", Lang::None), vec![("22/222/222/22".to_string(), InfoType::Ip, true)]);
        assert!(found_deep("22/222/222/22", Lang::None).is_empty(), "맥락 없는 인공 배열 + 표 구분자");
        for t in ["203 105 180 171", "210 / 229 / 211 / 26", "172\t188\t143\t7", "39,131,111,57"] {
            assert_eq!(found_deep(t, Lang::None), vec![(t.to_string(), InfoType::Ip, true)], "{t}");
        }
        assert_eq!(found_deep("서버 203 105 180 171", Lang::None), vec![("203 105 180 171".to_string(), InfoType::Ip, false)]);
        assert_eq!(found_deep("host=58/120/33/7", Lang::None), vec![("58/120/33/7".to_string(), InfoType::Ip, false)]);
        // 좌표·색상 맥락, 기호가 섞인 것, 표의 한 줄(5칸 이상)은 IP 아님
        for t in ["rgb(10, 43, 198, 163)", "좌표 115 116 75 44", "73,202~66-252", "10 43 198 163 77"] {
            assert!(found_deep(t, Lang::None).iter().all(|(_, k, _)| *k != InfoType::Ip), "{t}: {:?}", found_deep(t, Lang::None));
        }
        }

    /// IP주소만 켜면(심층 꺼짐, 기본) 점 표기만 검사 — 기호가 바뀐 IP 는 찾지 않는다
    #[test]
    fn ip_plain_is_dotted_only() {
        assert_eq!(found("서버 192.168.0.1 접속", Lang::None), vec![("192.168.0.1".to_string(), InfoType::Ip, false)]);
        for t in ["103^234^126^56", "서버 203 105 180 171", "192_168_0_1:8080", "ip=104^200^101^188", "211. 234. 10. 5"] {
            assert!(found(t, Lang::None).iter().all(|(_, k, _)| *k != InfoType::Ip), "{t}: {:?}", found(t, Lang::None));
        }
    }

    /// 전화번호 하이픈을 다른 기호 하나로 일괄 치환해도 확정
    #[test]
    fn uniform_substitution_phones() {
        for t in ["010+1234+5678", "010/1234/5678", "010|1234|5678", "010 1234 5678", "010^1234^5678", "010 -> 1234 -> 5678"] {
            assert_eq!(found(t, Lang::None), vec![(t.to_string(), InfoType::Phone, false)], "{t}");
        }
    }

    /// + 국가번호 표기는 해외핸드폰번호 (+82 10 포함)
    #[test]
    fn intl_phones() {
        for t in ["+82 10-1234-5678", "+821012345678", "+1 (555) 123-4567", "+44 20 7946 0958"] {
            assert_eq!(found(&format!("연락처 {t}"), Lang::None), vec![(t.to_string(), InfoType::IntlPhone, false)], "{t}");
        }
        let mut o = Options::default();
        o.enabled.remove(&InfoType::IntlPhone);
        assert!(found_with(o, "연락처 +82 10-1234-5678", Lang::None).iter().all(|(_, k, _)| *k != InfoType::IntlPhone));
    }

    /// 형식 제외: 같은 정보타입에서 숫자만 다른 값은 모두 제외
    #[test]
    fn excluded_formats() {
        let mut o = Options::default();
        o.excluded_formats.insert(crate::config::format_key(InfoType::Phone.label(), "010-1234-5678"));
        let got = found_with(o, "담당 010-1111-2222, 부담당 01033334444", Lang::None);
        assert_eq!(got, vec![("01033334444".to_string(), InfoType::Phone, false)]);
    }

    /// 주민번호: `-` 표기는 확정, 구분자 없음·공백은 치환 + 검토필요
    #[test]
    fn rrn_without_dash_is_review() {
        // 881111-1234569 : 날짜·성별·검증번호 모두 유효
        assert_eq!(found("주민 881111-1234569", Lang::None), vec![("881111-1234569".into(), InfoType::Rrn, false)]);
        assert_eq!(found("주민 8811111234569", Lang::None), vec![("8811111234569".into(), InfoType::Rrn, true)]);
        assert_eq!(found("주민 881111 1234569", Lang::None), vec![("881111 1234569".into(), InfoType::Rrn, true)]);
    }

    /// 구분자 없는 13자리: 성별에 맞는 공식의 체크섬 필수, 2020.10 이후 생년월(체크섬 없는 번호)은 검증 불가 → 번호 아님
    #[test]
    fn rrn_checksum_by_gender_and_issue_date() {
        // 보고된 오답: 날짜로 시작하는 문서번호 (2025년 생년월)
        for t in ["2507224910019", "2509183823119", "2511128021023", "2510237721138", "2508254911014"] {
            assert!(found(t, Lang::None).is_empty(), "{t}: {:?}", found(t, Lang::None));
        }
        // 내국인 공식만 맞는 외국인(성별 7), 외국인 공식만 맞는 내국인(성별 1)은 아님
        assert!(found("주민 9001017123455", Lang::None).iter().all(|(_, k, _)| *k != InfoType::Frn));
        // 2020.10 이후라도 `-` 표기는 날짜·성별만으로 검출 (체크섬 없는 번호)
        assert_eq!(found("주민 250722-4910019", Lang::None), vec![("250722-4910019".into(), InfoType::Rrn, false)]);
    }

    /// 정보가 없는 주석은 그대로, 정보가 있는 주석만 주석째 치환
    #[test]
    fn only_comments_with_info() {
        let t = "/// 상대 경로 (접기 키)\nlet x = 1; // d0 + f004\nlet y = 2; // 담당 010-1234-5678\n";
        let got = found(t, Lang::Rust);
        assert!(got.iter().all(|(v, ..)| !v.contains("상대 경로") && !v.contains("d0 + f004")), "{got:?}");
        assert!(got.iter().any(|(v, k, _)| *k == InfoType::Comment && v.contains("담당")), "{got:?}");
        assert!(got.iter().any(|(v, k, _)| *k == InfoType::Phone && v == "010-1234-5678"), "{got:?}");
    }
}

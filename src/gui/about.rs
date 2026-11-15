//! 정보 창 공용 데이터 — 개발자 정보와 사용한 오픈소스 목록.
//! 목록(`assets/licenses.tsv`)은 `python assets/make_licenses.py` 로 Cargo 의존성 그래프에서 생성한다.

pub const GITHUB: &str = "https://github.com/maspo1";
const LICENSES: &str = include_str!("../../assets/licenses.tsv");

/// (이름, 버전, 라이선스, 저장소, 용도)
pub(crate) fn crates() -> impl Iterator<Item = [&'static str; 5]> {
    LICENSES.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).map(|l| {
        let mut it = l.split('\t');
        std::array::from_fn(|_| it.next().unwrap_or(""))
    })
}

/// 실행 시 쓰는 OS 기능과 글꼴 (Cargo 의존성이 아닌 것)
pub(crate) const RUNTIME: &[(&str, &str, &str)] = &[
    ("Windows.Media.Ocr", "Windows 10/11 내장 OCR", "Microsoft — OS 구성 요소"),
    ("Vision (VNRecognizeTextRequest)", "macOS 내장 OCR", "Apple — OS 구성 요소"),
    ("AppKit", "macOS 화면", "Apple — OS 구성 요소"),
    ("맑은 고딕 / Apple SD 산돌고딕 Neo", "화면 한글 글꼴 (OS 설치본을 읽기만 함, 배포 안 함)", "Microsoft / Apple — OS 구성 요소"),
];

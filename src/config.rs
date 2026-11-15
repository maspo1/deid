//! 검사 옵션 (고급 사용자용 상세 조절 항목 포함)

use crate::common::InfoType;
use regex::Regex;
use std::collections::HashSet;

/// 사용자 정의 정규식 패턴
#[derive(Clone, Debug)]
pub struct CustomPattern {
    pub label: String,
    pub regex: Regex,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// 검사할 내장 정보 타입
    pub enabled: HashSet<InfoType>,
    /// 맥락 모델 채택 임계 확률 (AGENTS.md: p >= 0.7)
    pub threshold: f32,
    /// 사용자 정의 패턴 (그룹 1이 있으면 그룹 1만 치환)
    pub custom: Vec<CustomPattern>,
    /// 바이너리 내부 문자열 추출 최소 길이
    pub min_binary_run: usize,
    /// exe/dll 등 실행파일도 검사할지 (기본: 원본 그대로 복사)
    pub scan_executables: bool,
    /// 이보다 큰 파일은 검사하지 않고 복사 (바이트)
    pub max_file_size: u64,
    /// zip 안의 zip 등 중첩 처리 깊이
    pub max_depth: u32,
    /// 그림 속 글자 OCR 검사 (Windows 내장 OCR)
    pub ocr: bool,
    /// OCR 확대 배율 목록 (작은 글자 인식률 향상, 앞쪽 배율 결과 우선)
    pub ocr_scales: Vec<f32>,
    /// OCR 확대 후 화소 수 상한 — 큰 이미지의 처리 시간 제한
    pub ocr_max_pixels: u64,
    /// OCR 전에 흑백 이진화(오츠) — 화면 캡처의 작은 글자·점 인식률 향상 (교차 검증으로 기본 켬)
    pub ocr_enhance: bool,
    /// 자체 경량 OCR(숫자·영문 글자 단위 신경망, `ocr::lite`)로 1차 검사 — Windows OCR 은 중간 단계 이상에서만.
    /// 끄면 1차 검사도 Windows OCR (이전 방식)
    pub ocr_lite: bool,
    /// 심층 단계 — 0 기본(1차 검사), 1 중간(원본·이진화 이중 인식 + 배율 1.5), 2 최고(+ 막대 제거). 화면의 재검사가 올린다
    pub depth: u8,
    /// 흐림 강도 배수 (1.0 기본)
    pub blur_strength: f32,
    /// 파일/폴더 이름의 정보도 비식별화
    pub rename_files: bool,
    /// XGBoost 맥락 탐지 — 기호가 바뀐 전화번호·IP·카드·주민번호 (rules::ml)
    pub ml_context: bool,
    /// 확장자 선택 (고급) — 검사할 확장자 목록과 예외 규칙
    pub exts: crate::exts::ExtFilter,
    /// 사용자가 화면에서 제외한 값 — 검출된 원본 문자열과 완전히 같으면 치환하지 않는다
    pub excluded: HashSet<String>,
    /// 사용자가 화면에서 제외한 형식 (`format_key`: 정보타입 + 숫자를 #로 바꾼 모양) — 같은 형식은 모두 치환하지 않는다
    pub excluded_formats: HashSet<String>,
}

/// 형식 제외 키: 같은 정보타입(이름, 예: "IP주소")에서 숫자만 다르고 자리·기호·공백이 같은 값은 같은 형식
/// (`1 0 0 : 1` ≡ `2 5 5 : 3`). 화면은 보고서 레코드의 정보타입 이름으로 같은 키를 만든다.
pub fn format_key(label: &str, value: &str) -> String {
    let shape: String = value.trim().chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect();
    format!("{label}\t{shape}")
}

impl Default for Options {
    fn default() -> Self {
        Options {
            // IP주소(심층)은 기본 꺼짐 — 켜야 점 대신 다른 기호를 쓴 IP 까지 찾는다
            enabled: InfoType::BUILTIN.iter().filter(|t| **t != InfoType::IpDeep).cloned().collect(),
            threshold: 0.7,
            custom: Vec::new(),
            min_binary_run: 6,
            scan_executables: false,
            max_file_size: 512 * 1024 * 1024,
            max_depth: 3,
            ocr: true,
            ocr_scales: vec![3.0, 2.0],
            ocr_max_pixels: 16_000_000,
            ocr_enhance: true,
            ocr_lite: true,
            depth: 0,
            blur_strength: 1.0,
            rename_files: true,
            ml_context: true,
            exts: crate::exts::ExtFilter::default(),
            excluded: HashSet::new(),
            excluded_formats: HashSet::new(),
        }
    }
}

impl Options {
    pub fn is_on(&self, k: &InfoType) -> bool {
        match k {
            InfoType::Custom(_) => true,
            // 심층을 켜면 점 표기 IP 도 함께 검사
            InfoType::Ip => self.enabled.contains(k) || self.ip_deep(),
            _ => self.enabled.contains(k),
        }
    }

    /// IP주소(심층): 점 대신 다른 기호를 쓴 IP 를 표 검사·학습 모델로 찾음
    pub fn ip_deep(&self) -> bool {
        self.enabled.contains(&InfoType::IpDeep)
    }

    /// `라벨=정규식` 형식의 사용자 패턴 추가
    pub fn add_custom(&mut self, spec: &str) -> Result<(), String> {
        let (label, re) = spec
            .split_once('=')
            .ok_or_else(|| format!("패턴 형식 오류(라벨=정규식): {spec}"))?;
        let label = label.trim();
        if label.is_empty() {
            return Err(format!("패턴 라벨이 비어 있음: {spec}"));
        }
        let regex = Regex::new(re).map_err(|e| format!("정규식 오류 [{label}]: {e}"))?;
        self.custom.push(CustomPattern { label: label.to_string(), regex });
        Ok(())
    }

    /// 패턴 파일: 한 줄에 `라벨=정규식`, `#`으로 시작하면 무시
    pub fn load_pattern_file(&mut self, path: &std::path::Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            self.add_custom(l)?;
        }
        Ok(())
    }

    /// 쉼표 구분 키 목록으로 타입 끄기
    pub fn disable(&mut self, keys: &str) -> Result<(), String> {
        for k in keys.split(',').map(str::trim).filter(|k| !k.is_empty()) {
            let t = InfoType::from_key(k).ok_or_else(|| format!("알 수 없는 타입: {k}"))?;
            self.enabled.remove(&t);
        }
        Ok(())
    }

    /// 쉼표 구분 키 목록만 켜기
    pub fn only(&mut self, keys: &str) -> Result<(), String> {
        self.enabled.clear();
        for k in keys.split(',').map(str::trim).filter(|k| !k.is_empty()) {
            let t = InfoType::from_key(k).ok_or_else(|| format!("알 수 없는 타입: {k}"))?;
            self.enabled.insert(t);
        }
        Ok(())
    }
}

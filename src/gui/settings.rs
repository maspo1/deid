//! 화면 입력값 ↔ `Options` 변환과 검증.

use crate::common::InfoType;
use crate::config::Options;
use crate::exts::ExtFilter;
use std::path::PathBuf;

pub use crate::ipc::Job;

/// 화면에 묶인 설정 값
#[derive(Clone)]
pub struct Settings {
    pub input: String,
    pub output: String,
    /// InfoType::BUILTIN 순서
    pub enabled: [bool; InfoType::BUILTIN.len()],
    pub ml: bool,
    pub threshold: f32,
    pub ocr: bool,
    pub ocr_scales: String,
    /// 백만 화소
    pub ocr_mp: f32,
    pub ocr_enhance: bool,
    pub blur: f32,
    pub rename: bool,
    pub scan_exe: bool,
    pub max_mb: u64,
    pub min_run: usize,
    pub patterns: String,
    pub exts: ExtFilter,
}

impl Default for Settings {
    fn default() -> Self {
        let o = Options::default();
        Settings {
            input: String::new(),
            output: String::new(),
            enabled: std::array::from_fn(|i| o.is_on(&InfoType::BUILTIN[i])),
            ml: o.ml_context,
            threshold: o.threshold,
            ocr: o.ocr,
            ocr_scales: o.ocr_scales.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
            ocr_mp: o.ocr_max_pixels as f32 / 1_000_000.0,
            ocr_enhance: o.ocr_enhance,
            blur: o.blur_strength,
            rename: o.rename_files,
            scan_exe: o.scan_executables,
            max_mb: o.max_file_size / (1024 * 1024),
            min_run: o.min_binary_run,
            patterns: String::new(),
            exts: o.exts,
        }
    }
}

fn path_of(s: &str) -> Option<PathBuf> {
    let s = s.trim().trim_matches('"');
    (!s.is_empty()).then(|| PathBuf::from(s))
}

impl Settings {
    /// 사용자 정의 정규식 줄별 오류 (화면에 바로 표시)
    pub fn pattern_errors(&self) -> Vec<String> {
        let mut o = Options::default();
        self.patterns
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty() && !l.trim().starts_with('#'))
            .filter_map(|(n, l)| o.add_custom(l.trim()).err().map(|e| format!("{}번째 줄: {e}", n + 1)))
            .collect()
    }

    /// 검사 옵션 (폴더 입력과 무관한 부분)
    pub fn options(&self) -> Result<Options, String> {
        let mut o = Options::default();
        o.enabled = InfoType::BUILTIN.iter().zip(self.enabled).filter(|(_, on)| *on).map(|(k, _)| k.clone()).collect();
        o.ml_context = self.ml;
        o.threshold = self.threshold.clamp(0.0, 1.0);
        o.ocr = self.ocr;
        if o.ocr {
            o.ocr_scales = self
                .ocr_scales
                .split(',')
                .filter(|x| !x.trim().is_empty())
                .map(|x| x.trim().parse::<f32>().ok().filter(|v| (0.5..=6.0).contains(v)))
                .collect::<Option<Vec<_>>>()
                .filter(|v| !v.is_empty())
                .ok_or("OCR 배율은 0.5 ~ 6 사이 숫자를 쉼표로 구분해 입력하세요. 예: 3,2")?;
            o.ocr_max_pixels = (self.ocr_mp.clamp(1.0, 200.0) * 1_000_000.0) as u64;
            o.ocr_enhance = self.ocr_enhance;
        }
        o.blur_strength = self.blur.clamp(0.1, 10.0);
        o.rename_files = self.rename;
        o.scan_executables = self.scan_exe;
        o.max_file_size = self.max_mb.max(1) * 1024 * 1024;
        o.min_binary_run = self.min_run.max(3);
        o.exts = self.exts.clone();
        for (n, line) in self.patterns.lines().enumerate() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            o.add_custom(l).map_err(|e| format!("사용자 정의 정규식 {}번째 줄: {e}", n + 1))?;
        }
        if o.enabled.is_empty() && o.custom.is_empty() {
            return Err("검출 항목을 하나 이상 선택하거나 사용자 정의 정규식을 입력하세요.".into());
        }
        Ok(o)
    }

    /// 입력 검증 후 실행 정보 구성. 잘못된 입력은 사용자에게 보여줄 메시지로 돌려준다.
    pub fn job(&self) -> Result<Job, String> {
        let input = path_of(&self.input).ok_or("검사할 폴더를 선택하세요.")?;
        if !input.is_dir() {
            return Err(format!("검사할 폴더가 없습니다:\n{}", input.display()));
        }
        Ok(Job { input, output: path_of(&self.output), opts: self.options()? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_options() {
        let s = Settings::default();
        let o = s.options().unwrap();
        let d = Options::default();
        assert_eq!(o.enabled, d.enabled);
        assert_eq!(o.ocr_scales, d.ocr_scales);
        assert_eq!(o.max_file_size, d.max_file_size);
        assert!(s.job().is_err(), "폴더 없이 시작 불가");
    }

    #[test]
    fn pattern_errors_are_reported_per_line() {
        let s = Settings { patterns: "# 주석\n사번=EMP-\\d{6}\n잘못=(\n".into(), ..Default::default() };
        let e = s.pattern_errors();
        assert_eq!(e.len(), 1);
        assert!(e[0].starts_with("3번째 줄"), "{e:?}");
        assert!(s.options().is_err());
    }
}

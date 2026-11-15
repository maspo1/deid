//! 검사 결과 반영을 DB 트랜잭션처럼: 엔진은 임시 폴더(`<출력>~검토중`)에 쓰고,
//! 사용자가 화면에서 결과를 확인해 승인하면 출력 폴더로 옮기고(commit), 취소하면 임시 폴더만 지운다(rollback).
//! 임시 폴더는 출력 폴더 바로 옆이라 같은 드라이브 → 옮기기는 이름 바꾸기라서 즉시 끝난다.

use std::fs;
use std::path::{Path, PathBuf};

const STAGING_SUFFIX: &str = "~검토중";
/// 1차 검사 출력 표시 — 재검사까지 마치면 `_검사완료` 로 바뀐다
pub const FIRST_SUFFIX: &str = "_1번";
pub const DONE_SUFFIX: &str = "_검사완료";

/// 반영 대기 중인 검사 결과
pub struct Pending {
    pub staging: PathBuf,
    pub target: PathBuf,
    /// 같은 폴더를 다시 검사한 결과 — 합치지 않고 통째로 교체 (이름이 바뀐 파일의 옛 이름이 남지 않게)
    pub replace: bool,
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// 1차 검사 출력: `<검사 폴더>_1번`
pub fn first_output(input: &Path) -> PathBuf {
    with_suffix(input, FIRST_SUFFIX)
}

/// 재검사까지 마친 출력: `X_1번` → `X_검사완료` (그 밖의 이름은 뒤에 `_검사완료`)
pub fn final_output(first: &Path) -> PathBuf {
    let s = first.as_os_str().to_string_lossy();
    match s.strip_suffix(FIRST_SUFFIX) {
        Some(base) => PathBuf::from(format!("{base}{DONE_SUFFIX}")),
        None => with_suffix(first, DONE_SUFFIX),
    }
}

/// 재검사 동안 폴더의 검사 보고서(원본 값 기록)를 폴더 밖으로 빼 둔다 — 재검사가 보고서를 치환해 망가뜨리지 않게
pub fn set_aside_report(dir: &Path) -> Option<PathBuf> {
    let r = dir.join(crate::pipeline::REPORT_NAME);
    let aside = with_suffix(dir, "~보고서.csv");
    (r.is_file() && fs::rename(&r, &aside).is_ok()).then_some(aside)
}

/// 빼 둔 보고서를 `dir` 로 되돌림. 재검사가 새 보고서를 만들었으면 그 행을 뒤에 이어 붙인다
pub fn restore_report(aside: &Path, dir: &Path) {
    let r = dir.join(crate::pipeline::REPORT_NAME);
    let mut text = fs::read_to_string(aside).unwrap_or_default();
    if let Ok(new) = fs::read_to_string(&r) {
        for line in new.lines().skip(1) {
            if !text.ends_with('\n') && !text.is_empty() {
                text.push_str("\r\n");
            }
            text.push_str(line);
            text.push_str("\r\n");
        }
    }
    if dir.is_dir() && fs::write(&r, text).is_ok() {
        let _ = fs::remove_file(aside);
    }
}

pub fn staging_for(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(STAGING_SUFFIX);
    PathBuf::from(s)
}

/// 출력 폴더가 검사 폴더와 같거나 그 안이면 반영 시 원본을 덮어쓰게 되므로 거부
pub fn check_target(input: &Path, target: &Path) -> Result<(), String> {
    if target == input || target.starts_with(input) {
        return Err("출력 폴더는 검사 폴더 바깥이어야 합니다".into());
    }
    Ok(())
}

impl Pending {
    /// 승인: 임시 폴더 → 출력 폴더. 출력 폴더가 이미 있으면 기존 동작과 같이 같은 이름 파일을 덮어쓰며 합친다.
    pub fn commit(&self) -> Result<(), String> {
        if self.replace && self.target.exists() {
            let old = staging_for(&with_suffix(&self.target, "~이전"));
            discard(&old)?;
            fs::rename(&self.target, &old).map_err(|e| format!("반영 실패: {e}"))?;
            if let Err(e) = fs::rename(&self.staging, &self.target) {
                let _ = fs::rename(&old, &self.target); // 되돌림
                return Err(format!("반영 실패: {e}"));
            }
            return discard(&old);
        }
        if !self.target.exists() {
            return fs::rename(&self.staging, &self.target).map_err(|e| format!("반영 실패: {e}"));
        }
        merge(&self.staging, &self.target).map_err(|e| format!("반영 실패: {e}"))?;
        fs::remove_dir_all(&self.staging).map_err(|e| format!("임시 폴더 정리 실패: {e}"))
    }

    /// 취소: 임시 폴더만 삭제 (출력 폴더는 건드리지 않음)
    pub fn discard(&self) -> Result<(), String> {
        discard(&self.staging)
    }

    /// 임시 폴더 안의 경로 → 반영 후 경로
    pub fn map(&self, p: &Path) -> PathBuf {
        p.strip_prefix(&self.staging).map(|r| self.target.join(r)).unwrap_or_else(|_| p.to_path_buf())
    }
}

/// 이 모듈이 만든 임시 폴더(`~검토중` 으로 끝남)만 지운다
pub fn discard(staging: &Path) -> Result<(), String> {
    if !staging.as_os_str().to_string_lossy().ends_with(STAGING_SUFFIX) {
        return Err(format!("임시 폴더가 아닙니다: {}", staging.display()));
    }
    match fs::remove_dir_all(staging) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("임시 폴더 삭제 실패: {e}")),
    }
}

fn merge(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for e in fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            merge(&e.path(), &to)?;
        } else {
            if to.is_file() {
                fs::remove_file(&to)?;
            }
            fs::rename(e.path(), &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("deid_commit_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn pending(root: &Path) -> Pending {
        let target = root.join("out");
        let staging = staging_for(&target);
        fs::create_dir_all(staging.join("sub")).unwrap();
        fs::write(staging.join("a.txt"), "new-a").unwrap();
        fs::write(staging.join("sub/b.txt"), "new-b").unwrap();
        Pending { staging, target, replace: false }
    }

    #[test]
    fn commit_moves_into_new_target() {
        let root = tmp("new");
        let p = pending(&root);
        p.commit().unwrap();
        assert!(!p.staging.exists());
        assert_eq!(fs::read_to_string(p.target.join("sub/b.txt")).unwrap(), "new-b");
        assert_eq!(p.map(&p.staging.join("sub/b.txt")), p.target.join("sub/b.txt"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn commit_merges_over_existing_target() {
        let root = tmp("merge");
        let p = pending(&root);
        fs::create_dir_all(p.target.join("sub")).unwrap();
        fs::write(p.target.join("a.txt"), "old-a").unwrap();
        fs::write(p.target.join("keep.txt"), "keep").unwrap();
        p.commit().unwrap();
        assert!(!p.staging.exists());
        assert_eq!(fs::read_to_string(p.target.join("a.txt")).unwrap(), "new-a");
        assert_eq!(fs::read_to_string(p.target.join("keep.txt")).unwrap(), "keep");
        assert_eq!(fs::read_to_string(p.target.join("sub/b.txt")).unwrap(), "new-b");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discard_leaves_target_untouched() {
        let root = tmp("discard");
        let p = pending(&root);
        fs::create_dir_all(&p.target).unwrap();
        fs::write(p.target.join("a.txt"), "old-a").unwrap();
        p.discard().unwrap();
        assert!(!p.staging.exists());
        assert_eq!(fs::read_to_string(p.target.join("a.txt")).unwrap(), "old-a");
        // 임시 폴더가 아닌 경로는 지우지 않음
        assert!(discard(&p.target).is_err());
        assert!(p.target.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn names_and_replace_and_report() {
        assert_eq!(first_output(Path::new("C:/d/애플")), PathBuf::from("C:/d/애플_1번"));
        assert_eq!(final_output(Path::new("C:/d/애플_1번")), PathBuf::from("C:/d/애플_검사완료"));
        assert_eq!(final_output(Path::new("C:/d/out")), PathBuf::from("C:/d/out_검사완료"));

        // 제자리 교체: 옛 파일은 남지 않음
        let root = tmp("replace");
        let mut p = pending(&root);
        p.replace = true;
        fs::create_dir_all(&p.target).unwrap();
        fs::write(p.target.join("옛이름_010-1234-5678.txt"), "old").unwrap();
        p.commit().unwrap();
        assert!(!p.target.join("옛이름_010-1234-5678.txt").exists());
        assert_eq!(fs::read_to_string(p.target.join("a.txt")).unwrap(), "new-a");
        assert!(!p.staging.exists());

        // 보고서: 빼 두었다가 재검사 보고서 행을 이어 붙여 되돌림
        fs::write(p.target.join(crate::pipeline::REPORT_NAME), "h\r\n1차\r\n").unwrap();
        let aside = set_aside_report(&p.target).unwrap();
        assert!(!p.target.join(crate::pipeline::REPORT_NAME).exists());
        fs::write(p.target.join(crate::pipeline::REPORT_NAME), "h\r\n재검사\r\n").unwrap();
        restore_report(&aside, &p.target);
        assert_eq!(fs::read_to_string(p.target.join(crate::pipeline::REPORT_NAME)).unwrap(), "h\r\n1차\r\n재검사\r\n");
        assert!(!aside.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn target_must_be_outside_input() {
        let i = Path::new("C:/data/in");
        assert!(check_target(i, i).is_err());
        assert!(check_target(i, Path::new("C:/data/in/out")).is_err());
        assert!(check_target(i, Path::new("C:/data/in_검사완료")).is_ok());
    }
}

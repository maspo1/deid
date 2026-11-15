//! OS 기능 공통 창구 — 폴더/파일 선택, 탐색기(Finder) 열기, 오류 상자.
//! Windows 는 `win.rs`(공용 대화상자 직접 호출), 그 밖의 OS 는 rfd 를 쓴다.

use std::path::{Path, PathBuf};

/// 대화상자의 소유 창 (Windows: HWND 값, 그 밖: 사용 안 함)
#[derive(Clone, Copy, Default)]
pub struct Owner(#[cfg_attr(not(windows), allow(dead_code))] pub isize);

pub const APP_TITLE: &str = "deid - 개인정보 비식별화";

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::gui::win;
    use windows::Win32::Foundation::HWND;

    pub fn pick_folder(o: Owner, title: &str) -> Option<PathBuf> {
        win::pick_folder(HWND(o.0 as _), title)
    }
    pub fn pick_file(o: Owner, title: &str, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
        let specs: Vec<(String, String)> =
            filters.iter().map(|(n, e)| (n.to_string(), e.iter().map(|x| format!("*.{x}")).collect::<Vec<_>>().join(";"))).collect();
        let refs: Vec<(&str, &str)> = specs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        win::pick_file(HWND(o.0 as _), title, &refs)
    }
    pub fn open(p: &Path) {
        let _ = std::process::Command::new("explorer.exe").arg(p).spawn();
    }
    pub fn reveal(p: &Path) {
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(p.as_os_str());
        let _ = std::process::Command::new("explorer.exe").arg(arg).spawn();
    }
    pub fn message_box(msg: &str) {
        win::message_box(msg);
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn pick_folder(_: Owner, title: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title(title).pick_folder()
    }
    pub fn pick_file(_: Owner, title: &str, filters: &[(&str, &[&str])]) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new().set_title(title);
        for (n, e) in filters {
            if !e.contains(&"*") {
                d = d.add_filter(*n, e);
            }
        }
        d.pick_file()
    }
    fn opener() -> &'static str {
        if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        }
    }
    pub fn open(p: &Path) {
        let _ = std::process::Command::new(opener()).arg(p).spawn();
    }
    pub fn reveal(p: &Path) {
        if cfg!(target_os = "macos") {
            let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
        } else if let Some(d) = p.parent() {
            open(d);
        }
    }
    pub fn message_box(msg: &str) {
        rfd::MessageDialog::new().set_title(APP_TITLE).set_description(msg).set_level(rfd::MessageLevel::Error).show();
    }
}

pub use imp::{message_box, open, pick_file, pick_folder};

/// 파일을 선택한 상태로 폴더 열기 (파일이 없으면 가장 가까운 폴더)
pub fn reveal(p: &Path) {
    if p.is_file() {
        imp::reveal(p);
    } else if let Some(d) = p.ancestors().find(|x| x.is_dir()) {
        open(d);
    }
}

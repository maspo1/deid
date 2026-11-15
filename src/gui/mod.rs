//! 화면 공용 모듈 + macOS 화면.
//!
//! * Windows 화면(`crate::win32_gui`)과 같이 쓰는 것: `scan` 결과 모음, `settings` 입력값 ↔ Options,
//!   `sys` OS 기능(대화상자·Finder), `about` 오픈소스 목록
//! * `mac`: macOS 네이티브 화면 (AppKit, objc2)
//! * `win`: Windows 전용 도우미

pub(crate) mod about;
#[cfg(target_os = "macos")]
mod mac;
pub(crate) mod scan;
pub(crate) mod settings;
pub(crate) mod sys;
#[cfg(windows)]
mod win;

#[cfg(target_os = "macos")]
pub use mac::run;
#[cfg(windows)]
pub use win::use_cli_console;

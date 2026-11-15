//! 진입점.
//! * 인자 없이 실행(더블클릭) → GUI (Windows: 네이티브 Win32 `win32_gui`, macOS: egui `gui`)
//! * 콘솔에서 인자와 함께 실행 → CLI (Windows: 부모 콘솔에 붙어서 출력)
//! * 탐색기에서 폴더를 exe 아이콘에 끌어다 놓기 → 그 폴더가 채워진 GUI
//! * `--engine` → GUI 가 띄우는 검사 엔진 프로세스 (`deid::ipc`)

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--engine") {
        return deid::ipc::engine_main();
    }
    #[cfg(windows)]
    {
        if args.is_empty() || !deid::gui::use_cli_console() {
            return deid::win32_gui::run(args.first().map(std::path::PathBuf::from));
        }
        println!();
    }
    #[cfg(not(windows))]
    {
        if args.is_empty() {
            return deid::gui::run(None);
        }
    }
    deid::cli::main(args)
}

//! Windows 전용 예제 — 코드는 `win.rs` (Windows 기본 글꼴·GDI 사용).
//! 다른 OS 에서도 `cargo test`(예제까지 컴파일)가 통과하도록 진입점만 나눴다.

#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::main();
    #[cfg(not(windows))]
    eprintln!("이 예제는 Windows 전용입니다 (Windows 기본 글꼴·GDI 사용).");
}

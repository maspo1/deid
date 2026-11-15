//! Windows 전용 도우미 — 공용 파일/폴더 선택 대화상자(IFileOpenDialog), 오류 상자, 콘솔 연결.

use std::path::PathBuf;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};

use super::sys::APP_TITLE;

/// 창을 열기 전/실패 시 쓰는 기본 오류 상자
pub fn message_box(msg: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    unsafe {
        MessageBoxW(HWND::default(), &HSTRING::from(msg), &HSTRING::from(APP_TITLE), MB_OK | MB_ICONERROR);
    }
}

/// CLI 로 실행할 수 있는 환경인가.
/// * 출력이 파일/파이프로 리디렉션됨 → 그 핸들을 그대로 사용 (콘솔에 붙으면 핸들이 바뀌므로 붙지 않음)
/// * 콘솔(cmd, PowerShell)에서 실행됨 → 부모 콘솔에 붙어서 출력
/// * 탐색기에서 실행됨 → false (GUI)
pub fn use_cli_console() -> bool {
    unsafe {
        let redirected = GetStdHandle(STD_OUTPUT_HANDLE).map_or(false, |h| !h.is_invalid());
        redirected || AttachConsole(ATTACH_PARENT_PROCESS).is_ok()
    }
}

/// 공용 열기 대화상자 (폴더 선택 또는 파일 선택). 취소하면 None.
fn open_dialog(owner: HWND, title: &str, folder: bool, filters: &[(&str, &str)]) -> Option<PathBuf> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let mut opts = dlg.GetOptions().ok()? | FOS_FORCEFILESYSTEM;
        if folder {
            opts |= FOS_PICKFOLDERS;
        } else {
            opts |= FOS_FILEMUSTEXIST;
        }
        dlg.SetOptions(opts).ok()?;
        dlg.SetTitle(&HSTRING::from(title)).ok()?;
        // 필터 문자열은 대화상자가 떠 있는 동안 살아 있어야 함
        let names: Vec<HSTRING> = filters.iter().map(|f| HSTRING::from(f.0)).collect();
        let specs: Vec<HSTRING> = filters.iter().map(|f| HSTRING::from(f.1)).collect();
        let spec: Vec<COMDLG_FILTERSPEC> = names
            .iter()
            .zip(&specs)
            .map(|(n, s)| COMDLG_FILTERSPEC { pszName: PCWSTR(n.as_ptr()), pszSpec: PCWSTR(s.as_ptr()) })
            .collect();
        if !spec.is_empty() {
            dlg.SetFileTypes(&spec).ok()?;
        }
        dlg.Show(owner).ok()?;
        let item = dlg.GetResult().ok()?;
        let p = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

pub fn pick_folder(owner: HWND, title: &str) -> Option<PathBuf> {
    open_dialog(owner, title, true, &[])
}

pub fn pick_file(owner: HWND, title: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
    open_dialog(owner, title, false, filters)
}

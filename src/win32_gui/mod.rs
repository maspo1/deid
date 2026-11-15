//! 네이티브 Win32 GUI (Windows 10/11 x64) — `windows` 크레이트만 사용.
//!
//! egui 판(`src/gui`, macOS 화면)과 같은 기능을 공용 컨트롤로 구현한 Windows 기본 화면.
//! 검사 엔진은 egui 판과 같은 별도 프로세스(`crate::ipc`)를 쓰고, 설정 검증(`gui::settings`)·결과 저장(`gui::scan`)·
//! 대화상자(`gui::sys`)도 그대로 재사용한다.
//!
//! 구조
//! * `mod.rs`     창 클래스·메시지 루프·앱 상태·메뉴·경로 줄·상태줄·로그 탭
//! * `options`    왼쪽 옵션 패널 (스크롤)
//! * `results`    검출 결과 탭 (가상 리스트뷰 + 필터)
//! * `forensic`   파일 분석 탭 (가상 트리 리스트뷰 + [+] + 상세 창), 모델은 `tree`
//! * `preview`    미리보기 탭
//! * `commit`     검사 결과를 임시 폴더에 받아 두고, 화면에서 확인·승인하면 출력 폴더로 반영 (취소하면 버림)
//! * `exclude`    검출 값 제외(체크 해제·우클릭) — 같은 값은 모든 파일에서 함께, 반영 시 엔진이 치환하지 않음
//! * `notice`     알림 창 (검사 완료 → 재검사 카운트다운, 재검사 결과 안전/경고)
//! * `ext_dlg`    확장자 선택 창, `about` 정보 창 (`logo` GitHub 마크)
//! * `gdi`        GDI 핸들 RAII, `dpi` 모니터별 DPI·글꼴, `ctl`/`listview` 컨트롤 공통, `layout` 배치 계산
//!
//! 안전 규칙
//! * 앱 상태는 `Box<App>` 하나, 창의 GWLP_USERDATA 에 보관하고 WM_NCDESTROY 에서 해제
//! * 메시지 처리 중 같은 스레드로 되돌아 들어오는 메시지(컨트롤이 보내는 알림 등)는 `with_app` 이 막는다 —
//!   `&mut App` 이 동시에 둘 생기지 않는다. 모달 동작(대화상자·메뉴)은 처리를 끝낸 뒤 `Act` 로 실행한다.

mod about;
mod commit;
mod ctl;
mod dpi;
mod exclude;
mod ext_dlg;
mod forensic;
mod gdi;
mod layout;
mod listview;
mod logo;
mod notice;
mod options;
mod preview;
mod results;
mod tree;
mod worker;

use crate::gui::scan::{Scan, Src};
use crate::gui::settings::Settings;
use crate::gui::sys::{self, Owner};
use ctl::{send, FontReg};
use dpi::{px, FontKind, Fonts};
use layout::R;
use std::cell::Cell;
use std::path::PathBuf;
use std::process::ExitCode;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetSysColorBrush, ScreenToClient, SetBkMode, SetTextColor, COLOR_BTNFACE, HDC, TRANSPARENT};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

/// 엔진 출력 도착 알림
const WM_ENGINE: u32 = WM_APP + 1;
/// 화면 갱신 묶기 (엔진 메시지가 많아도 0.1초에 한 번만 다시 그림)
const TIMER_REFRESH: usize = 1;

/// 컨트롤·메뉴 ID
mod id {
    pub const IN_EDIT: u16 = 101;
    pub const IN_BTN: u16 = 102;
    pub const OUT_EDIT: u16 = 103;
    pub const OUT_BTN: u16 = 104;
    pub const TAB: u16 = 105;
    pub const START: u16 = 106;
    pub const REPORT: u16 = 107;
    pub const OUTDIR: u16 = 108;
    pub const DISCARD: u16 = 109;
    pub const LOG: u16 = 120;
    pub const R_REVIEW: u16 = 130;
    pub const R_KIND: u16 = 131;
    pub const R_SEARCH: u16 = 132;
    pub const R_LIST: u16 = 133;
    pub const F_EXPAND: u16 = 140;
    pub const F_COLLAPSE: u16 = 141;
    pub const F_FOUND: u16 = 142;
    pub const F_SEARCH: u16 = 143;
    pub const F_TREE: u16 = 144;
    pub const F_TAB_RECS: u16 = 145;
    pub const F_TAB_PROPS: u16 = 146;
    pub const F_RECS: u16 = 147;
    pub const F_PROPS: u16 = 148;
    pub const F_SRC: u16 = 149;
    pub const F_DST: u16 = 150;
    pub const P_INPUT: u16 = 160;
    pub const P_LIST: u16 = 162;
    pub const M_OPEN: u16 = 300;
    pub const M_REPORT: u16 = 301;
    pub const M_OUTDIR: u16 = 302;
    pub const M_EXIT: u16 = 303;
    pub const M_TAB0: u16 = 310; // ~313
    pub const M_EXTS: u16 = 320;
    pub const M_CLEAR_EXCL: u16 = 321;
    pub const M_ABOUT: u16 = 330;
    pub const C_SRC: u16 = 340;
    pub const C_DST: u16 = 341;
    pub const C_COPY: u16 = 342;
    pub const C_EXCL: u16 = 343;
    pub const C_INCL: u16 = 344;
    pub const C_EXCL_FMT: u16 = 345;
    /// 알림 창 버튼
    pub const N_VERIFY: u16 = 360;
    pub const N_CLOSE: u16 = 361;
}

const TABS: [&str; 4] = ["로그", "검출 결과", "파일 분석", "미리보기"];
const T_LOG: usize = 0;
const T_RESULTS: usize = 1;
const T_FORENSIC: usize = 2;
const T_PREVIEW: usize = 3;

/// 메인 창 컨트롤
#[derive(Default)]
struct Ctl {
    in_label: HWND,
    in_edit: HWND,
    in_btn: HWND,
    out_label: HWND,
    out_edit: HWND,
    out_btn: HWND,
    tab: HWND,
    progress: HWND,
    btn_discard: HWND,
    btn_report: HWND,
    btn_out: HWND,
    btn_start: HWND,
    status: HWND,
    log: HWND,
    menu: HMENU,
}

/// 처리를 마친 뒤(재진입 보호 밖에서) 실행할 모달·외부 동작
enum Act {
    PickInput,
    PickOutput,
    PickPatterns,
    Error(String),
    ConfirmClose,
    /// 반영하지 않은 결과가 있을 때 닫기
    ConfirmClosePending,
    /// 제외 항목이 있는 채로 승인 (건수)
    ConfirmExcluded(usize),
    /// 알림 창
    Notice(notice::Spec),
    /// 우클릭 메뉴 (화면 좌표)
    Popup(POINT, Vec<(u16, &'static str)>),
    OpenExt,
    OpenAbout,
    Open(PathBuf),
    Reveal(PathBuf),
    Destroy,
}

pub struct App {
    hwnd: HWND,
    fonts: Fonts,
    reg: FontReg,
    c: Ctl,
    panel: Option<options::Panel>,
    s: Settings,
    engine: Option<worker::Engine>,
    stopping: bool,
    close_after: bool,
    scan: Option<Scan>,
    /// 실행 중인 검사의 임시 폴더·출력 폴더
    staging: Option<commit::Pending>,
    /// 검사는 끝났고 승인을 기다리는 결과 (임시 폴더)
    pending: Option<commit::Pending>,
    /// 사용자가 제외한 값 (프로그램 메모리에만 보관)
    excluded: std::collections::HashSet<String>,
    /// 사용자가 제외한 형식 (`config::format_key`) — 엔진에 넘겨 다음 검사에서 숫자만 다른 값도 제외.
    /// 화면 표시는 지금 결과에서 같은 형식인 값을 `excluded` 에 함께 넣어 기존 값 제외 그대로 쓴다
    excluded_formats: std::collections::HashSet<String>,
    /// 지금 결과를 만든 엔진에 넘긴 제외 목록
    applied: std::collections::HashSet<String>,
    /// 다시 검사하는 동안 들고 있는 이전 결과 (제외 행 이어받기)
    carry: Option<Scan>,
    /// 제외를 적용한 재검사가 끝나면 바로 반영
    auto_commit: bool,
    /// 지금 결과를 만든 작업 설정 (제외 적용 재검사에 그대로 사용)
    last_job: Option<crate::ipc::Job>,
    /// 체크 상자 이미지 (DPI 별, 이전 목록은 Drop 으로 해제)
    checks: Option<gdi::ImageList>,
    /// 우클릭 메뉴의 제외 대상 값
    ctx_values: Vec<String>,
    /// 제외 목록이 속한 검사 폴더 — 다른 폴더를 검사하면 제외 목록을 비운다
    excl_root: Option<PathBuf>,
    /// 마지막으로 반영한 출력 폴더 (재검사 대상)
    done_target: Option<PathBuf>,
    /// 재검사 진행 중
    rescan: Option<Rescan>,
    /// 폴더별 최고 단계 심층검사 횟수와 그때 남은 정보 (프로그램 메모리에만)
    top_hist: std::collections::HashMap<PathBuf, (u32, Vec<String>)>,
    notice: Option<notice::Dlg>,
    notice_spec: Option<notice::Spec>,
    timer: bool,
    tab: usize,
    res: results::Page,
    fx: forensic::Page,
    pv: preview::Page,
    ext: Option<ext_dlg::Dlg>,
    about: Option<about::Dlg>,
    /// 가상 목록 칸 글자 버퍼 (다음 알림까지 유효)
    buf: Vec<u16>,
    /// 우클릭 메뉴 대상
    ctx_target: Option<(Src, usize)>,
    acts: Vec<Act>,
}

/// 재검사 (재검사까지 마쳐야 검사완료)
struct Rescan {
    /// 1 중간, 2 최고
    depth: u8,
    /// 재검사한 폴더 (1번 폴더, 또는 제자리 검사한 검사완료 폴더)
    input: PathBuf,
    /// 검사완료 폴더
    done: PathBuf,
    /// 재검사 동안 빼 둔 보고서
    aside: Option<PathBuf>,
    /// 이번 재검사에서 최고 단계를 몇 번째 하는지
    top_runs: u32,
}

/// 재검사에서 남은 정보가 있을 때 다음 단계
#[derive(Debug, PartialEq)]
enum Next {
    /// 치환해 반영했으니 최고 단계로 다시 (이번 재검사에서 몇 번째 최고 단계인지)
    Top(u32),
    /// 최고 단계를 2번 해도 남음 → 경고, 사용자가 직접 고치도록
    Warn,
}

/// `depth`: 방금 끝난 단계, `top_runs`: 이번 재검사에서 최고 단계를 몇 번째로 했는지,
/// `runs`: 이 폴더에서 최고 단계를 한 총 횟수(프로그램 메모리), `same`: 남은 정보가 지난 최고 단계와 그대로인지
fn rescan_next(depth: u8, top_runs: u32, runs: u32, same: bool) -> Next {
    if depth < 2 {
        Next::Top(1)
    } else if top_runs >= 2 || (runs >= 2 && same) {
        Next::Warn
    } else {
        Next::Top(top_runs + 1)
    }
}

fn folder_name(p: &std::path::Path) -> String {
    p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())
}

fn depth_name(d: u8) -> &'static str {
    match d {
        0 => "기본",
        1 => "중간 단계",
        _ => "최고 단계",
    }
}

thread_local! {
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// 앱 상태를 빌려 처리. 처리 중 되돌아 들어온 메시지는 None (기본 처리로 넘어감).
fn with_app<T>(hwnd: HWND, f: impl FnOnce(&mut App) -> T) -> Option<T> {
    if BUSY.with(|b| b.get()) {
        return None;
    }
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut App;
    if p.is_null() {
        return None;
    }
    BUSY.with(|b| b.set(true));
    // 안전: p 는 이 창이 소유한 Box<App>, 이 스레드에서만 접근하고 BUSY 로 중복 대여를 막는다
    let r = f(unsafe { &mut *p });
    BUSY.with(|b| b.set(false));
    Some(r)
}

/// 쌓인 동작 실행 (모달 대화상자 등은 여기서 — 이때 다른 메시지도 정상 처리된다)
fn drain_acts(hwnd: HWND) {
    while let Some(act) = with_app(hwnd, |a| (!a.acts.is_empty()).then(|| a.acts.remove(0))).flatten() {
        perform(hwnd, act);
    }
}

fn owner(hwnd: HWND) -> Owner {
    Owner(hwnd.0 as isize)
}

fn perform(hwnd: HWND, act: Act) {
    match act {
        Act::PickInput => {
            if let Some(p) = sys::pick_folder(owner(hwnd), "검사할 폴더 선택") {
                with_app(hwnd, |a| a.set_input(&p.display().to_string()));
            }
        }
        Act::PickOutput => {
            if let Some(p) = sys::pick_folder(owner(hwnd), "출력 폴더 선택 (검사할 폴더 바깥)") {
                with_app(hwnd, |a| {
                    a.s.output = p.display().to_string();
                    ctl::set_text(a.c.out_edit, &a.s.output);
                });
            }
        }
        Act::PickPatterns => {
            let filters: [(&str, &[&str]); 2] = [("패턴 파일", &["txt", "pat", "cfg"]), ("모든 파일", &["*"])];
            if let Some(p) = sys::pick_file(owner(hwnd), "사용자 정의 정규식 파일 (줄마다 라벨=정규식)", &filters) {
                match std::fs::read(&p) {
                    Ok(b) => {
                        let t = crate::common::decode_text(&b).map(|(t, _)| t).unwrap_or_else(|| String::from_utf8_lossy(&b).into_owned());
                        with_app(hwnd, |a| {
                            let cur = a.s.patterns.trim_end().to_string();
                            a.s.patterns = if cur.is_empty() { t } else { format!("{cur}\n{t}") };
                            if let Some(p) = &a.panel {
                                p.write(&a.s);
                            }
                        });
                    }
                    Err(e) => perform(hwnd, Act::Error(format!("파일을 읽을 수 없습니다: {e}"))),
                }
            }
        }
        Act::Error(e) => {
            ctl::message(hwnd, &e, MB_OK | MB_ICONWARNING);
        }
        Act::ConfirmClose => {
            let r = ctl::message(hwnd, "검사 중입니다. 중지하고 닫을까요?\n(출력 폴더는 불완전한 상태로 남습니다)", MB_YESNO | MB_ICONQUESTION);
            if r == IDYES {
                with_app(hwnd, |a| {
                    a.close_after = true;
                    a.stop();
                });
            }
        }
        Act::ConfirmClosePending => {
            let n = with_app(hwnd, |a| a.excluded_count()).unwrap_or(0);
            let extra = if n > 0 { format!("\n(제외된 항목 {n}개는 치환하지 않고 반영합니다)") } else { String::new() };
            let r = ctl::message(
                hwnd,
                &format!("검사 결과를 아직 출력 폴더에 반영하지 않았습니다.\n\n예: 반영하고 닫기{extra}\n아니요: 반영하지 않고(버리고) 닫기\n취소: 돌아가기"),
                MB_YESNOCANCEL | MB_ICONQUESTION,
            );
            if r == IDYES || r == IDNO {
                let closed = with_app(hwnd, |a| {
                    if r == IDNO {
                        return a.discard_pending();
                    }
                    a.close_after = true; // 제외 적용 재검사가 필요하면 반영이 끝난 뒤 닫힘
                    let done = a.approve_now();
                    if !done && !a.running() {
                        a.close_after = false; // 반영 실패 → 닫지 않음
                    }
                    done
                })
                .unwrap_or(false);
                if closed {
                    perform(hwnd, Act::Destroy);
                }
            }
        }
        Act::ConfirmExcluded(n) => {
            let rescan = with_app(hwnd, |a| a.needs_rescan()).unwrap_or(false);
            let how = if rescan { "\n이를 위해 제외를 적용해 다시 검사한 뒤 자동으로 반영합니다." } else { "" };
            let r = ctl::message(
                hwnd,
                &format!("제외된 항목이 {n}개 있습니다.\n\n제외한 값은 치환하지 않고 원본 그대로 출력 폴더에 반영합니다.{how}\n\n계속할까요?"),
                MB_OKCANCEL | MB_ICONWARNING,
            );
            if r == IDOK {
                with_app(hwnd, |a| a.approve_now());
            }
        }
        Act::Popup(pt, items) => {
            let m = gdi::PopupMenu::new(&items);
            let cmd = unsafe { TrackPopupMenu(m.handle(), TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, None) };
            if cmd.0 != 0 {
                with_app(hwnd, |a| a.on_context(cmd.0 as u16));
            }
        }
        Act::OpenExt => ext_dlg::open(hwnd),
        Act::OpenAbout => about::open(hwnd),
        Act::Notice(spec) => notice::open(hwnd, spec),
        Act::Open(p) => sys::open(&p),
        Act::Reveal(p) => sys::reveal(&p),
        Act::Destroy => unsafe {
            let _ = DestroyWindow(hwnd);
        },
    }
}

pub fn run(prefill: Option<PathBuf>) -> ExitCode {
    dpi::enable_per_monitor_v2();
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_LISTVIEW_CLASSES | ICC_TAB_CLASSES | ICC_BAR_CLASSES | ICC_PROGRESS_CLASS | ICC_LINK_CLASS,
        };
        let _ = InitCommonControlsEx(&icc);
    }
    register_classes();
    let dpi = unsafe { GetDpiForSystem() };
    let mut s = Settings::default();
    if let Some(p) = prefill.filter(|p| p.is_dir()) {
        s.input = p.display().to_string();
    }
    let app = Box::new(App::new(s, dpi));
    let raw = Box::into_raw(app);
    let title = ctl::wide(sys::APP_TITLE);
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("deid_main"),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            px(1120, dpi),
            px(760, dpi),
            None,
            None,
            ctl::hinst(),
            Some(raw as *const core::ffi::c_void),
        )
    };
    let Ok(hwnd) = hwnd else {
        // 창이 안 만들어졌으면 WM_NCDESTROY 도 오지 않으므로 여기서 해제
        drop(unsafe { Box::from_raw(raw) });
        sys::message_box("창을 만들 수 없습니다.");
        return ExitCode::FAILURE;
    };
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWDEFAULT);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // 탭 키 이동·기본 버튼 (현재 최상위 창 기준)
            let top = GetAncestor(msg.hwnd, GA_ROOT);
            if !top.is_invalid() && IsDialogMessageW(top, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    ExitCode::SUCCESS
}

fn register_classes() {
    let icon = unsafe { LoadIconW(ctl::hinst(), PCWSTR(1 as _)) }.unwrap_or_default(); // assets/app.rc 의 IDI_APP
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
    let bg = windows::Win32::Graphics::Gdi::HBRUSH((COLOR_BTNFACE.0 + 1) as isize as _);
    let classes: [(PCWSTR, WNDPROC); 5] = [
        (w!("deid_main"), Some(main_proc)),
        (options::CLASS, Some(panel_proc)),
        (ext_dlg::CLASS, Some(ext_dlg::proc_)),
        (about::CLASS, Some(about::proc_)),
        (notice::CLASS, Some(notice::proc_)),
    ];
    for (name, proc_) in classes {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: proc_,
            hInstance: ctl::hinst(),
            hIcon: icon,
            hIconSm: icon,
            hCursor: cursor,
            hbrBackground: bg,
            lpszClassName: name,
            ..Default::default()
        };
        unsafe {
            RegisterClassExW(&wc);
        }
    }
}

extern "system" fn main_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCCREATE => {
                let cs = &*(lp.0 as *const CREATESTRUCTW);
                let app = cs.lpCreateParams as *mut App;
                (*app).hwnd = hwnd;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            WM_NCDESTROY => {
                let p = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut App;
                if !p.is_null() {
                    drop(Box::from_raw(p)); // 엔진 프로세스·글꼴·이미지 목록이 여기서 정리됨
                }
                PostQuitMessage(0);
                return LRESULT(0);
            }
            _ => {}
        }
        let r = with_app(hwnd, |a| a.handle(msg, wp, lp)).flatten();
        drain_acts(hwnd);
        r.unwrap_or_else(|| DefWindowProcW(hwnd, msg, wp, lp))
    }
}

/// 옵션 패널: 스크롤만 직접 처리하고 나머지 알림은 메인 창으로 넘긴다
extern "system" fn panel_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let parent = GetParent(hwnd).unwrap_or_default();
        match msg {
            WM_COMMAND | WM_HSCROLL | WM_NOTIFY | WM_CTLCOLORSTATIC => SendMessageW(parent, msg, wp, lp),
            WM_VSCROLL | WM_MOUSEWHEEL => {
                with_app(parent, |a| {
                    let dpi = a.fonts.dpi;
                    if let Some(p) = &mut a.panel {
                        p.on_scroll(msg, wp.0, dpi);
                    }
                });
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}

fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

pub(super) fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

impl App {
    fn new(s: Settings, dpi: u32) -> App {
        App {
            hwnd: HWND::default(),
            fonts: Fonts::new(dpi),
            reg: FontReg::default(),
            c: Ctl::default(),
            panel: None,
            s,
            engine: None,
            stopping: false,
            close_after: false,
            scan: None,
            staging: None,
            pending: None,
            excluded: Default::default(),
            excluded_formats: Default::default(),
            applied: Default::default(),
            carry: None,
            auto_commit: false,
            last_job: None,
            checks: None,
            ctx_values: Vec::new(),
            excl_root: None,
            done_target: None,
            rescan: None,
            top_hist: Default::default(),
            notice: None,
            notice_spec: None,
            timer: false,
            tab: T_LOG,
            res: Default::default(),
            fx: Default::default(),
            pv: Default::default(),
            ext: None,
            about: None,
            buf: Vec::new(),
            ctx_target: None,
            acts: Vec::new(),
        }
    }

    fn dpi(&self) -> u32 {
        self.fonts.dpi
    }

    fn running(&self) -> bool {
        self.engine.is_some()
    }

    // ───────────────────────────── 생성·배치 ─────────────────────────────

    fn create(&mut self) {
        let h = self.hwnd;
        let dpi = unsafe { GetDpiForWindow(h) };
        if dpi != self.fonts.dpi {
            self.fonts = Fonts::new(dpi);
        }
        self.c.menu = build_menu();
        unsafe {
            let _ = SetMenu(h, self.c.menu);
            DragAcceptFiles(h, true);
        }
        let reg = &mut self.reg;
        self.c.in_label = ctl::label(h, "검사 폴더", reg, FontKind::Ui);
        self.c.in_edit = ctl::edit(h, id::IN_EDIT, 0, reg, FontKind::Ui);
        self.c.in_btn = ctl::button(h, "찾아보기...", id::IN_BTN, reg);
        self.c.out_label = ctl::label(h, "출력 폴더", reg, FontKind::Ui);
        self.c.out_edit = ctl::edit(h, id::OUT_EDIT, 0, reg, FontKind::Ui);
        self.c.out_btn = ctl::button(h, "찾아보기...", id::OUT_BTN, reg);
        ctl::cue(self.c.in_edit, "폴더를 선택하거나 창에 끌어다 놓으세요");

        self.c.tab = ctl::child(h, WC_TABCONTROLW, "", WS_CLIPSIBLINGS.0 | WS_TABSTOP.0, 0, id::TAB, reg, FontKind::Ui);
        for (i, t) in TABS.iter().enumerate() {
            let mut w = ctl::wide(t);
            let it = TCITEMW { mask: TCIF_TEXT, pszText: windows::core::PWSTR(w.as_mut_ptr()), ..Default::default() };
            send(self.c.tab, TCM_INSERTITEMW, i, &it as *const _ as isize);
        }
        self.c.progress = ctl::child(h, PROGRESS_CLASSW, "", 0, 0, 0, reg, FontKind::Ui);
        self.c.btn_discard = ctl::button(h, "반영 취소", id::DISCARD, reg);
        self.c.btn_report = ctl::button(h, "보고서 열기", id::REPORT, reg);
        self.c.btn_out = ctl::button(h, "출력 폴더 열기", id::OUTDIR, reg);
        self.c.btn_start = ctl::child(h, ctl::BUTTON, "검사 시작", WS_TABSTOP.0 | 1 /* BS_DEFPUSHBUTTON */, 0, id::START, reg, FontKind::Bold);
        self.c.status = ctl::child(h, STATUSCLASSNAMEW, "", SBARS_SIZEGRIP, 0, 0, reg, FontKind::Ui);

        // 탭 페이지 (메인 창의 자식, 탭 컨트롤 위에 겹침)
        self.c.log = listview::create(h, id::LOG, true, LVS_NOCOLUMNHEADER, &[listview::col("", 0)], dpi, reg, FontKind::Mono);
        self.results_create();
        self.forensic_create();
        self.preview_create();
        self.panel = Some(options::Panel::create(h, &mut self.reg));
        unsafe {
            // 탭 컨트롤을 맨 아래로 → 페이지 컨트롤이 위에 그려진다
            let _ = SetWindowPos(self.c.tab, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }

        self.reg.apply(&self.fonts);
        self.forensic_images();
        self.check_images();
        if let Some(p) = &self.panel {
            p.write(&self.s);
        }
        ctl::set_text(self.c.in_edit, &self.s.input);
        self.update_out_cue();
        self.show_tab(T_LOG);
        self.refresh();
        self.layout();
    }

    fn client(&self) -> (i32, i32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        (rc.right, rc.bottom)
    }

    /// 탭 컨트롤 안쪽(페이지) 영역
    fn page_rect(&self, tab: R) -> R {
        let mut rc = RECT { left: tab.x, top: tab.y, right: tab.right(), bottom: tab.bottom() };
        send(self.c.tab, TCM_ADJUSTRECT, 0, &mut rc as *mut _ as isize);
        let m = px(2, self.dpi());
        R::new(rc.left + m, rc.top + m, rc.right - rc.left - 2 * m, rc.bottom - rc.top - 2 * m)
    }

    fn layout(&mut self) {
        let (w, h) = self.client();
        if w == 0 || h == 0 {
            return;
        }
        send(self.c.status, WM_SIZE, 0, 0);
        let mut sr = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.c.status, &mut sr);
        }
        let sb_h = sr.bottom - sr.top;
        let dpi = self.dpi();
        let m = layout::main(w, h, sb_h, dpi);
        let c = &self.c;
        for (hw, r) in [
            (c.in_label, m.in_label),
            (c.in_edit, m.in_edit),
            (c.in_btn, m.in_btn),
            (c.out_label, m.out_label),
            (c.out_edit, m.out_edit),
            (c.out_btn, m.out_btn),
            (c.tab, m.tab),
            (c.progress, m.progress),
            (c.btn_discard, m.btn_discard),
            (c.btn_report, m.btn_report),
            (c.btn_out, m.btn_out),
            (c.btn_start, m.btn_start),
        ] {
            ctl::place(hw, r);
        }
        if let Some(p) = &mut self.panel {
            p.layout(m.panel, dpi);
        }
        let parts = [w - px(320, dpi), w - px(170, dpi), -1];
        send(self.c.status, SB_SETPARTS, parts.len(), parts.as_ptr() as isize);
        let pg = self.page_rect(m.tab);
        ctl::place(self.c.log, pg);
        listview::fit_columns(self.c.log, &[listview::col("", 0)], pg.w, dpi);
        self.results_layout(pg);
        self.forensic_layout(pg);
        self.preview_layout(pg);
    }

    fn show_tab(&mut self, t: usize) {
        self.tab = t;
        send(self.c.tab, TCM_SETCURSEL, t, 0);
        ctl::show(self.c.log, t == T_LOG);
        self.results_show(t == T_RESULTS);
        self.forensic_show(t == T_FORENSIC);
        self.preview_show(t == T_PREVIEW);
        if t == T_PREVIEW {
            self.preview_update();
        }
        for i in 0..TABS.len() {
            let flag = if i == t { MF_CHECKED } else { MF_UNCHECKED };
            unsafe {
                CheckMenuItem(self.c.menu, (id::M_TAB0 + i as u16) as u32, flag.0);
            }
        }
    }

    fn on_dpi(&mut self, dpi: u32, suggested: &RECT) {
        self.fonts = Fonts::new(dpi); // 이전 글꼴은 Drop 으로 해제 (컨트롤에 새 글꼴을 입힌 뒤)
        self.reg.apply(&self.fonts);
        if let Some(d) = &self.ext {
            d.apply_fonts(&self.fonts);
        }
        if let Some(d) = &self.about {
            d.apply_fonts(&self.fonts);
        }
        if let Some(d) = &self.notice {
            d.apply_fonts(&self.fonts);
        }
        self.forensic_images();
        self.check_images();
        self.results_columns();
        self.forensic_columns();
        self.preview_columns();
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.layout();
    }

    // ───────────────────────────── 상태 갱신 ─────────────────────────────

    fn set_input(&mut self, s: &str) {
        self.s.input = s.to_string();
        ctl::set_text(self.c.in_edit, s);
        self.update_out_cue();
    }

    fn update_out_cue(&self) {
        let hint = match self.s.input.trim().trim_matches('"') {
            "" => "비워 두면 <검사 폴더>_1번 (재검사까지 마치면 _검사완료)".to_string(),
            i => format!("비워 두면 {} (재검사까지 마치면 _검사완료)", commit::first_output(&std::path::absolute(i).unwrap_or(i.into())).display()),
        };
        ctl::cue(self.c.out_edit, &hint);
    }

    fn output_dir(&self) -> Option<PathBuf> {
        self.scan.as_ref()?.finished.as_ref()?.output_dir.clone()
    }

    fn report(&self) -> Option<PathBuf> {
        self.scan.as_ref()?.finished.as_ref()?.report.clone()
    }

    /// 상태줄·진행 막대·버튼·탭 제목·각 목록 개수
    fn refresh(&mut self) {
        let running = self.running();
        let (files, recs, review) = self.scan.as_ref().map_or((0, 0, 0), |s| (s.entries.len(), s.all_records().count(), s.all_records().filter(|r| r.review).count()));
        // 진행 막대: 목록 준비 중엔 움직이는 막대
        let preparing = self.scan.as_ref().is_some_and(|s| s.running() && s.total.is_none());
        let style = unsafe { GetWindowLongPtrW(self.c.progress, GWL_STYLE) } as u32;
        let want = if preparing { style | PBS_MARQUEE } else { style & !PBS_MARQUEE };
        if want != style {
            unsafe {
                SetWindowLongPtrW(self.c.progress, GWL_STYLE, want as isize);
            }
        }
        send(self.c.progress, PBM_SETMARQUEE, preparing as usize, 30);
        let (done, total) = match &self.scan {
            Some(s) if s.running() => (s.done, s.total.unwrap_or(0)),
            Some(s) if s.finished.as_ref().is_some_and(|f| f.ok) => (1, 1),
            _ => (0, 1),
        };
        send(self.c.progress, PBM_SETRANGE32, 0, total.max(1) as isize);
        send(self.c.progress, PBM_SETPOS, done, 0);

        let status = if self.stopping {
            "중지하는 중... (처리 중인 파일이 끝나면 멈춥니다)".to_string()
        } else if self.pending.is_some() {
            let n = self.excluded_count();
            let ex = if n > 0 { format!(" · 제외 {n}건") } else { String::new() };
            format!("검토 대기 (아직 반영 안 됨){ex} — 결과를 확인하고 [승인 — 반영] 또는 [반영 취소]를 누르세요")
        } else {
            self.scan.as_ref().map_or_else(|| "준비".to_string(), |s| s.status_line())
        };
        self.status_text(0, &status);
        self.status_text(1, &format!("파일 {files}개"));
        self.status_text(2, &if review > 0 { format!("검출 {recs}건 (검토 {review})") } else { format!("검출 {recs}건") });

        let pending = self.pending.is_some();
        ctl::set_text(self.c.btn_start, if running { "중지" } else if pending { "승인 — 반영" } else { "검사 시작" });
        ctl::show(self.c.btn_discard, pending);
        ctl::enable(self.c.btn_start, !self.stopping);
        let (rep, out) = (self.report().filter(|_| !running), self.output_dir().filter(|_| !running));
        ctl::enable(self.c.btn_report, rep.is_some());
        ctl::enable(self.c.btn_out, out.is_some());
        for (mid, on) in [(id::M_REPORT, rep.is_some()), (id::M_OUTDIR, out.is_some()), (id::M_OPEN, !running), (id::M_CLEAR_EXCL, !running && !(self.excluded.is_empty() && self.excluded_formats.is_empty()))] {
            unsafe {
                let _ = EnableMenuItem(self.c.menu, mid as u32, if on { MF_ENABLED } else { MF_GRAYED });
            }
        }
        for hw in [self.c.in_edit, self.c.in_btn, self.c.out_edit, self.c.out_btn] {
            ctl::enable(hw, !running);
        }
        if let Some(p) = &self.panel {
            p.set_enabled(!running, &self.s);
        }

        let titles = [
            "로그".to_string(),
            if review > 0 { format!("검출 결과 ({recs}, 검토 {review})") } else { format!("검출 결과 ({recs})") },
            format!("파일 분석 ({files})"),
            "미리보기".to_string(),
        ];
        for (i, t) in titles.iter().enumerate() {
            let mut w = ctl::wide(t);
            let it = TCITEMW { mask: TCIF_TEXT, pszText: windows::core::PWSTR(w.as_mut_ptr()), ..Default::default() };
            send(self.c.tab, TCM_SETITEMW, i, &it as *const _ as isize);
        }

        // 로그: 끝으로 자동 스크롤
        let n = self.scan.as_ref().map_or(0, |s| s.log.len());
        listview::set_count(self.c.log, n);
        if n > 0 {
            listview::ensure_visible(self.c.log, n - 1);
        }
        self.results_refresh();
        self.forensic_refresh();
    }

    fn status_text(&self, part: usize, s: &str) {
        let w = ctl::wide(s);
        send(self.c.status, SB_SETTEXTW, part, w.as_ptr() as isize);
    }

    // ───────────────────────────── 검사 ─────────────────────────────

    fn start(&mut self) {
        let job = match self.s.job() {
            Ok(j) => j,
            Err(e) => return self.acts.push(Act::Error(e)),
        };
        let mut job = job;
        let root = std::path::absolute(&job.input).unwrap_or(job.input.clone());
        // 다른 폴더를 검사하면 이전 폴더에서 제외해 둔 값은 잊는다 (정보가 다를 수 있음)
        let forgot = if self.excl_root.as_ref().is_some_and(|r| *r != root) {
            self.excluded_formats.clear();
            std::mem::take(&mut self.excluded).len()
        } else {
            0
        };
        self.excl_root = Some(root.clone());
        job.opts.excluded = self.excluded.clone();
        job.opts.excluded_formats = self.excluded_formats.clone();
        // 엔진은 임시 폴더에 쓰고, 출력 폴더는 승인할 때만 바뀐다
        let target = match &job.output {
            Some(o) => std::path::absolute(o).unwrap_or(o.clone()),
            None => commit::first_output(&root),
        };
        if let Err(e) = commit::check_target(&root, &target) {
            return self.acts.push(Act::Error(e));
        }
        if let Some(p) = self.pending.take() {
            let _ = p.discard(); // 반영하지 않은 이전 결과는 버림
        }
        let staging = commit::staging_for(&target);
        job.output = Some(staging.clone());
        self.launch(job, commit::Pending { staging, target, replace: false }, false);
        if forgot > 0 {
            if let Some(s) = &mut self.scan {
                s.log.push(format!("새 검사 폴더 — 이전 폴더에서 제외해 둔 값 {forgot}개는 잊었습니다."));
            }
        }
    }

    /// 재검사 (검사완료로 인정받는 단계): 검사 폴더를 `input` 으로 바꿔 심층 단계 `depth` 로 검사하고, 끝나면 `done` 에 자동 반영.
    /// 1번 폴더 → 검사완료 폴더(중간), 또는 검사완료 폴더를 제자리에서(최고).
    fn start_rescan(&mut self, depth: u8, input: PathBuf, done: PathBuf, top_runs: u32) {
        let Some(last) = self.last_job.as_ref() else { return };
        if self.running() || !input.is_dir() {
            return;
        }
        let mut opts = last.opts.clone();
        opts.depth = depth;
        opts.excluded = self.excluded.clone(); // 사용자가 일부러 남긴 값은 재검사에서도 그대로
        opts.excluded_formats = self.excluded_formats.clone();
        let aside = commit::set_aside_report(&input);
        let staging = commit::staging_for(&done);
        let job = crate::ipc::Job { input: input.clone(), output: Some(staging.clone()), opts };
        // 화면의 검사 폴더·출력 폴더도 재검사 대상으로 바꿈
        self.set_input(&input.display().to_string());
        self.s.output = done.display().to_string();
        ctl::set_text(self.c.out_edit, &self.s.output);
        let replace = input == done;
        self.rescan = Some(Rescan { depth, input: input.clone(), done: done.clone(), aside, top_runs });
        self.launch(job, commit::Pending { staging, target: done, replace }, true);
        if self.engine.is_none() {
            self.rescan_abort();
            return;
        }
        if let Some(s) = &mut self.scan {
            s.log.insert(0, format!("재검사 — {} 심층검사 (검사완료로 인정받는 단계)", depth_name(depth)));
        }
        self.show_tab(T_LOG);
    }

    /// 재검사가 반영되지 못하고 끝남: 빼 둔 보고서를 원래 폴더로
    fn rescan_abort(&mut self) {
        if let Some(r) = self.rescan.take() {
            if let Some(a) = &r.aside {
                commit::restore_report(a, &r.input);
            }
        }
    }

    /// 재검사 반영 직후: 남은 정보가 없으면 안전, 있으면 (치환해 반영한 뒤) 최고 단계로 다시, 최고 단계를 2번 해도 그대로면 경고
    fn after_rescan(&mut self, r: Rescan) {
        if let Some(a) = &r.aside {
            commit::restore_report(a, &r.done);
        }
        // 1번 폴더는 검사완료 폴더로 대체됐으므로 지움 (이 프로그램이 만든 `_1번` 폴더만)
        if r.input != r.done && r.input.to_string_lossy().ends_with(commit::FIRST_SUFFIX) && self.done_target.as_ref() == Some(&r.input) {
            if std::fs::remove_dir_all(&r.input).is_ok() {
                if let Some(s) = &mut self.scan {
                    s.log.push(format!("1번 폴더는 검사완료 폴더로 바뀌었습니다: {}", r.done.display()));
                }
            }
        }
        self.done_target = Some(r.done.clone());
        // 검사 폴더 칸은 마지막으로 검사를 마친 폴더(검사완료), 출력 칸은 비움
        self.set_input(&r.done.display().to_string());
        self.s.output.clear();
        ctl::set_text(self.c.out_edit, "");
        let report = r.input.join(crate::pipeline::REPORT_NAME);
        let mut left: Vec<String> = self
            .scan
            .iter()
            .flat_map(|s| s.entries.iter().filter(|e| e.path != report).flat_map(|e| e.records.iter().map(move |x| format!("{}\t{}", e.path.display(), x.original))))
            .chain(self.scan.iter().flat_map(|s| s.names.iter().flat_map(|n| n.records.iter().map(move |x| format!("{}\t{}", n.path.display(), x.original)))))
            .collect();
        left.sort();
        let kept = self.excluded.len();
        if left.is_empty() {
            if let Some(s) = &mut self.scan {
                s.log.push(format!("✔ 검사완료 — 재검사({})에서 남은 정보 0건: 안전합니다. ({})", depth_name(r.depth), r.done.display()));
            }
            let note = if kept > 0 { format!("\n(사용자가 제외해 남긴 값 {kept}개는 세지 않았습니다)") } else { String::new() };
            self.acts.push(Act::Notice(notice::Spec {
                icon: notice::Icon::Ok,
                title: "검사완료 — 안전합니다".into(),
                text: format!("재검사({} 심층검사)에서 남은 개인정보·민감정보가 없습니다.{note}\n\n검사완료 폴더: {}", depth_name(r.depth), folder_name(&r.done)),
                buttons: vec![(id::N_CLOSE, "확인".into())],
                countdown: None,
            }));
            return;
        }
        let n = left.len();
        // 최고 단계: 같은 폴더 횟수와 남은 정보가 그대로인지 (프로그램 메모리)
        let (runs, same) = if r.depth >= 2 {
            let h = self.top_hist.entry(r.done.clone()).or_default();
            h.0 += 1;
            let same = h.1 == left;
            h.1 = left;
            (h.0, same)
        } else {
            (0, false)
        };
        let next = rescan_next(r.depth, r.top_runs, runs, same);
        if let Next::Top(k) = next {
            if let Some(s) = &mut self.scan {
                s.log.push(format!("재검사({})에서 남은 정보 {n}건을 찾아 치환했습니다 → 최고 단계 심층검사로 다시 확인합니다.", depth_name(r.depth)));
            }
            return self.start_rescan(2, r.done.clone(), r.done, k);
        }
        {
            if let Some(s) = &mut self.scan {
                s.log.push(format!("⚠ 최고 단계 심층검사를 {runs}번 했지만 정보 {n}건이 남아 있습니다{} — 직접 확인해 고쳐 주세요.", if same { " (변화 없음)" } else { "" }));
            }
            self.show_tab(T_RESULTS);
            self.acts.push(Act::Notice(notice::Spec {
                icon: notice::Icon::Warn,
                title: format!("정보 {n}건이 계속 남아 있습니다"),
                text: format!(
                    "같은 폴더를 최고 단계로 {runs}번 심층검사했지만{} 정보가 남아 있습니다.\n자동으로 치환할 수 없는 형식일 수 있습니다.\n[검출 결과] 탭에서 위치를 확인하고 원본을 직접 고쳐 주세요.\n\n폴더: {}",
                    if same { " 결과가 바뀌지 않고" } else { "" },
                    folder_name(&r.done)
                ),
                buttons: vec![(id::N_CLOSE, "확인".into())],
                countdown: None,
            }));
            return;
        }
    }

    /// 엔진 실행 (임시 폴더에 씀). `auto_commit` 이면 끝나는 대로 반영
    fn launch(&mut self, job: crate::ipc::Job, p: commit::Pending, auto_commit: bool) {
        if let Err(e) = commit::discard(&p.staging) {
            return self.acts.push(Act::Error(e));
        }
        let root = std::path::absolute(&job.input).unwrap_or(job.input.clone());
        let mut scan = Scan::new(root);
        scan.log.push(format!("검사 시작: {}", job.input.display()));
        scan.log.push(format!(
            "XGBoost 맥락 탐지 {}, 임계값 {:.2}, 검사 확장자 {}/{}개{}, 사용자 정규식 {}개",
            if job.opts.ml_context { "켜짐" } else { "꺼짐" },
            job.opts.threshold,
            job.opts.exts.enabled.len(),
            crate::exts::catalog().len(),
            if job.opts.exts.scan_unknown { " + 목록 외 확장자" } else { "" },
            job.opts.custom.len()
        ));
        if !job.opts.excluded.is_empty() {
            scan.log.push(format!("제외 목록 {}개 값은 검출·치환하지 않습니다", job.opts.excluded.len()));
        }
        match worker::spawn(&job, self.hwnd, WM_ENGINE) {
            Ok(e) => {
                self.engine = Some(e);
                self.staging = Some(p);
                self.stopping = false;
                self.carry = self.scan.replace(scan);
                self.applied = job.opts.excluded.clone();
                self.auto_commit = auto_commit;
                self.last_job = Some(job);
                self.fx.reset();
                // 결과가 바뀌었으므로 필터 캐시도 버림 (새 결과의 version 이 우연히 같으면 옛 행 번호로 그려 범위를 벗어남)
                self.res.reset();
                if self.tab == T_PREVIEW {
                    self.show_tab(T_LOG);
                }
                self.refresh();
            }
            Err(e) => self.acts.push(Act::Error(e)),
        }
    }

    /// [승인 — 반영]: 제외 항목이 있으면 먼저 건수를 알린다
    fn request_approve(&mut self) {
        match self.excluded_count() {
            0 => {
                self.approve_now();
            }
            n => self.acts.push(Act::ConfirmExcluded(n)),
        }
    }

    /// 반영 실행. 제외 상태가 엔진 결과와 다르면 제외를 적용해 다시 검사한 뒤 자동 반영 (그때는 false)
    fn approve_now(&mut self) -> bool {
        if !self.needs_rescan() {
            return self.approve();
        }
        let (Some(p), Some(mut job)) = (self.pending.take(), self.last_job.take()) else { return self.approve() };
        job.opts.excluded = self.excluded.clone();
        job.opts.excluded_formats = self.excluded_formats.clone();
        self.launch(job, p, true);
        false
    }

    fn excluded_count(&self) -> usize {
        self.scan.as_ref().map_or(0, |s| exclude::count(s, &self.excluded))
    }

    fn needs_rescan(&self) -> bool {
        self.scan.as_ref().is_some_and(|s| exclude::needs_rescan(s, &self.excluded, &self.applied))
    }

    /// 체크 상자 누름·스페이스 (검토 대기 중에만 — 반영된 결과는 바꿀 수 없음)
    pub(super) fn toggle_values(&mut self, vals: &[String]) {
        if vals.is_empty() {
            return;
        }
        if self.pending.is_none() || self.running() {
            self.status_text(0, "제외는 검토 대기 중에만 바꿀 수 있습니다 (반영된 결과는 바꿀 수 없음)");
            return;
        }
        exclude::toggle(&mut self.excluded, vals);
        let back: Vec<String> = vals.iter().filter(|v| !self.excluded.contains(*v)).cloned().collect();
        self.drop_formats(&back);
        self.exclusion_changed();
    }

    /// 되살린 값의 형식 제외도 푼다 (형식이 남아 있으면 다시 검사할 때 또 제외되므로)
    fn drop_formats(&mut self, vals: &[String]) {
        let Some(scan) = &self.scan else { return };
        for r in scan.all_records().filter(|r| vals.contains(&r.original)) {
            self.excluded_formats.remove(&crate::config::format_key(r.base(), &r.original));
        }
    }

    /// 이 형식은 제외: 고른 값들의 형식을 기억하고, 지금 결과에서 같은 형식인 값을 모두 제외
    fn exclude_formats(&mut self, vals: &[String]) {
        let Some(scan) = &self.scan else { return };
        let key = |r: &crate::ipc::Rec| crate::config::format_key(r.base(), &r.original);
        self.excluded_formats.extend(scan.all_records().filter(|r| vals.contains(&r.original)).map(key));
        let same: Vec<String> = scan.all_records().filter(|r| self.excluded_formats.contains(&key(r))).map(|r| r.original.clone()).collect();
        self.excluded.extend(same);
    }

    /// 우클릭 메뉴: 제외/제외 취소 항목 (검토 대기 중에만)
    pub(super) fn exclude_menu(&mut self, vals: Vec<String>) -> Vec<(u16, &'static str)> {
        let mut m = Vec::new();
        if self.pending.is_some() && !self.running() && !vals.is_empty() {
            if vals.iter().any(|v| !self.excluded.contains(v)) {
                m.push((id::C_EXCL, "제외 — 치환하지 않음 (같은 값 모두)"));
            }
            m.push((id::C_EXCL_FMT, "이 형식은 제외 — 숫자만 다른 값 모두"));
            if vals.iter().any(|v| self.excluded.contains(v)) {
                m.push((id::C_INCL, "제외 취소 — 다시 치환"));
            }
        }
        self.ctx_values = vals;
        m
    }

    /// 제외 상태가 바뀌면 세 목록의 체크·글자색과 요약·상태줄만 다시 그림 (행 목록은 그대로)
    fn exclusion_changed(&mut self) {
        for h in [self.res.list, self.fx.tree, self.fx.recs] {
            listview::redraw(h);
        }
        self.results_refresh();
        self.refresh();
    }

    fn check_images(&mut self) {
        let list = gdi::check_boxes(px(16, self.dpi()));
        for h in [self.res.list, self.fx.tree, self.fx.recs] {
            listview::use_states(h, list.handle());
        }
        self.checks = Some(list);
    }

    /// 승인: 임시 폴더 → 출력 폴더. 성공하면 true
    fn approve(&mut self) -> bool {
        let Some(p) = self.pending.take() else { return true };
        if let Err(e) = p.commit() {
            self.pending = Some(p);
            self.acts.push(Act::Error(e));
            return false;
        }
        if let Some(s) = &mut self.scan {
            for e in &mut s.entries {
                e.out = p.map(&e.out);
            }
            if let Some(f) = &mut s.finished {
                f.output_dir = f.output_dir.as_deref().map(|d| p.map(d));
                f.report = f.report.as_deref().map(|r| p.map(r));
            }
            s.log.push(format!("✔ 반영 완료 — 출력 폴더: {}", p.target.display()));
            s.version += 1;
        }
        if let Some(r) = self.rescan.take() {
            self.after_rescan(r);
        } else {
            self.done_target = Some(p.target.clone());
            if !self.close_after {
                self.acts.push(Act::Notice(notice::Spec {
                    icon: notice::Icon::Info,
                    title: "1차 검사 완료".into(),
                    text: format!(
                        "출력 폴더: {}\n\n재검사까지 마쳐야 검사완료로 인정됩니다.\n재검사는 이 폴더를 심층 검사해 남은 정보를 치환하고\n폴더 이름을 _검사완료 로 바꿉니다.",
                        folder_name(&p.target)
                    ),
                    buttons: vec![(id::N_VERIFY, "재검사".into()), (id::N_CLOSE, "닫기".into())],
                    countdown: Some((id::N_VERIFY, 30)),
                }));
            }
        }
        self.refresh();
        true
    }

    /// 반영 취소: 임시 폴더만 지움 (출력 폴더는 그대로). 성공하면 true
    fn discard_pending(&mut self) -> bool {
        let Some(p) = self.pending.take() else { return true };
        if let Err(e) = p.discard() {
            self.pending = Some(p);
            self.acts.push(Act::Error(e));
            return false;
        }
        if let Some(s) = &mut self.scan {
            if let Some(f) = &mut s.finished {
                f.output_dir = None;
                f.report = None;
            }
            s.log.push(format!("✖ 반영 취소 — 출력 폴더({})는 바뀌지 않았습니다.", p.target.display()));
            s.version += 1;
        }
        self.refresh();
        true
    }

    fn stop(&mut self) {
        if let Some(e) = &mut self.engine {
            e.stop();
            self.stopping = true;
            self.refresh();
        }
    }

    /// 엔진 메시지를 결과에 반영. 화면 갱신은 타이머로 묶는다.
    fn on_engine(&mut self) {
        let (Some(e), Some(scan)) = (&self.engine, &mut self.scan) else { return };
        let mut finished = false;
        while let Ok(ev) = e.rx.try_recv() {
            finished |= scan.apply(ev);
        }
        if finished {
            self.engine = None; // 엔진 프로세스 정리
            self.stopping = false;
            let failed = self.scan.as_ref().and_then(|s| s.finished.as_ref()).is_some_and(|f| !f.ok);
            let carry = self.carry.take();
            let auto = std::mem::take(&mut self.auto_commit);
            if let Some(p) = self.staging.take() {
                if failed {
                    let _ = p.discard(); // 중지·실패: 불완전한 임시 결과는 버림 (출력 폴더는 그대로)
                    self.rescan_abort();
                } else {
                    if let (Some(prev), Some(s)) = (&carry, &mut self.scan) {
                        exclude::carry_over(prev, s, &self.applied);
                    }
                    self.pending = Some(p);
                    if auto {
                        if let (Some(s), None) = (&mut self.scan, &self.rescan) {
                            s.log.push(format!("제외 {}건을 적용해 다시 검사했습니다 — 바로 반영합니다.", exclude::count(s, &self.applied)));
                        }
                        self.approve();
                    } else if let (Some(s), Some(p)) = (&mut self.scan, &self.pending) {
                        s.log.push(format!("▶ 검토 대기 — 아직 출력 폴더({})에 반영하지 않았습니다.", p.target.display()));
                        s.log.push("  결과를 확인한 뒤 [승인 — 반영]을 누르면 반영되고, [반영 취소]를 누르면 버립니다. (출력 폴더 열기·보고서 열기는 임시 결과를 엽니다)".into());
                        s.log.push("  치환하지 않을 값은 [검출 결과]·[파일 분석] 탭에서 체크를 해제하거나 우클릭 → 제외 하세요. (같은 값은 함께 제외)".into());
                    }
                }
            }
            if failed {
                if let Some(l) = self.scan.as_ref().and_then(|s| s.log.last().cloned()) {
                    if !self.close_after {
                        self.acts.push(Act::Error(l));
                    }
                }
            } else if self.tab == T_LOG && self.scan.as_ref().is_some_and(|s| s.all_records().next().is_some()) {
                self.show_tab(T_RESULTS);
            }
            self.refresh();
            if self.close_after {
                self.acts.push(Act::Destroy);
            }
        } else if !self.timer {
            self.timer = true;
            unsafe {
                SetTimer(self.hwnd, TIMER_REFRESH, 100, None);
            }
        }
    }

    // ───────────────────────────── 메시지 ─────────────────────────────

    fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_CREATE => {
                self.create();
                Some(LRESULT(0))
            }
            WM_SIZE => {
                self.layout();
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                let mi = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
                mi.ptMinTrackSize = POINT { x: px(layout::MIN_W, self.dpi()), y: px(layout::MIN_H, self.dpi()) };
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let rc = unsafe { *(lp.0 as *const RECT) };
                self.on_dpi(loword(wp.0) as u32, &rc);
                Some(LRESULT(0))
            }
            WM_COMMAND => {
                self.on_command(loword(wp.0), hiword(wp.0) as u32);
                Some(LRESULT(0))
            }
            WM_HSCROLL => {
                // 옵션 패널의 슬라이더
                self.read_panel();
                Some(LRESULT(0))
            }
            WM_NOTIFY => self.on_notify(lp),
            WM_CTLCOLORSTATIC => self.on_ctlcolor(HDC(wp.0 as _), HWND(lp.0 as _)),
            WM_DROPFILES => {
                self.on_drop(HDROP(wp.0 as _));
                Some(LRESULT(0))
            }
            WM_ENGINE => {
                self.on_engine();
                Some(LRESULT(0))
            }
            WM_TIMER if wp.0 == TIMER_REFRESH => {
                unsafe {
                    let _ = KillTimer(self.hwnd, TIMER_REFRESH);
                }
                self.timer = false;
                self.refresh();
                Some(LRESULT(0))
            }
            WM_CLOSE => {
                self.acts.push(if self.running() {
                    Act::ConfirmClose
                } else if self.pending.is_some() {
                    Act::ConfirmClosePending
                } else {
                    Act::Destroy
                });
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP => {
                let pt = POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 };
                self.on_splitter(msg, pt).then_some(LRESULT(0))
            }
            WM_SETCURSOR if self.tab == T_FORENSIC => {
                let mut pt = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut pt);
                    let _ = ScreenToClient(self.hwnd, &mut pt);
                }
                if self.fx_splitter_hit(pt) {
                    unsafe {
                        SetCursor(LoadCursorW(None, IDC_SIZENS).unwrap_or_default());
                    }
                    return Some(LRESULT(1));
                }
                None
            }
            WM_DESTROY => {
                self.engine = None; // 엔진 종료를 기다린 뒤 임시 폴더 정리
                for p in [self.staging.take(), self.pending.take()].into_iter().flatten() {
                    let _ = p.discard(); // 승인하지 않은 결과는 남기지 않음
                }
                self.rescan_abort();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn read_panel(&mut self) {
        if let Some(p) = &self.panel {
            p.read(&mut self.s);
            p.set_enabled(!self.running(), &self.s);
        }
        if self.tab == T_PREVIEW {
            self.preview_update();
        }
    }

    fn on_command(&mut self, cid: u16, code: u32) {
        use options::oid;
        match cid {
            id::IN_EDIT if code == EN_CHANGE => {
                self.s.input = ctl::text(self.c.in_edit);
                self.update_out_cue();
            }
            id::OUT_EDIT if code == EN_CHANGE => self.s.output = ctl::text(self.c.out_edit),
            id::IN_BTN | id::M_OPEN => self.acts.push(Act::PickInput),
            id::OUT_BTN => self.acts.push(Act::PickOutput),
            id::START => {
                if self.running() {
                    self.stop();
                } else if self.pending.is_some() {
                    self.request_approve();
                } else {
                    self.start();
                }
            }
            id::DISCARD => {
                self.discard_pending();
            }
            id::N_VERIFY => {
                if let Some(first) = self.done_target.clone() {
                    let done = commit::final_output(&first);
                    self.start_rescan(1, first, done, 0);
                }
            }
            id::N_CLOSE => {}
            id::REPORT | id::M_REPORT => {
                if let Some(p) = self.report() {
                    self.acts.push(Act::Open(p));
                }
            }
            id::OUTDIR | id::M_OUTDIR => {
                if let Some(p) = self.output_dir() {
                    self.acts.push(Act::Open(p));
                }
            }
            id::M_EXIT => {
                send(self.hwnd, WM_CLOSE, 0, 0);
            }
            x if (id::M_TAB0..id::M_TAB0 + 4).contains(&x) => self.show_tab((x - id::M_TAB0) as usize),
            id::M_EXTS | oid::EXTS => self.acts.push(Act::OpenExt),
            id::M_ABOUT => self.acts.push(Act::OpenAbout),
            id::M_CLEAR_EXCL => {
                self.excluded.clear();
                self.excluded_formats.clear();
                self.exclusion_changed();
                self.refresh();
            }
            oid::ALL | oid::NONE => {
                self.s.enabled = [cid == oid::ALL; crate::common::InfoType::BUILTIN.len()];
                if let Some(p) = &self.panel {
                    p.write(&self.s);
                }
                self.read_panel();
            }
            oid::LOAD => self.acts.push(Act::PickPatterns),
            oid::TRY => self.show_tab(T_PREVIEW),
            oid::DEFAULTS => {
                let keep = (self.s.input.clone(), self.s.output.clone(), self.s.patterns.clone());
                self.s = Settings { input: keep.0, output: keep.1, patterns: keep.2, ..Default::default() };
                if let Some(p) = &self.panel {
                    p.write(&self.s);
                }
                self.read_panel();
            }
            x if (oid::TYPE0..=oid::THR).contains(&x) && (code == BN_CLICKED || code == EN_CHANGE) => self.read_panel(),
            _ => {
                self.results_command(cid, code);
                self.forensic_command(cid, code);
                self.preview_command(cid, code);
            }
        }
    }

    fn on_notify(&mut self, lp: LPARAM) -> Option<LRESULT> {
        let nm = unsafe { &*(lp.0 as *const NMHDR) };
        let cid = nm.idFrom as u16;
        match (cid, nm.code) {
            (id::TAB, TCN_SELCHANGE) => {
                let t = send(self.c.tab, TCM_GETCURSEL, 0, 0).max(0) as usize;
                self.show_tab(t);
                Some(LRESULT(0))
            }
            (id::LOG, LVN_GETDISPINFOW) => {
                let di = unsafe { &mut *(lp.0 as *mut NMLVDISPINFOW) };
                if (di.item.mask & LVIF_TEXT).0 != 0 {
                    let line = self.scan.as_ref().and_then(|s| s.log.get(di.item.iItem as usize)).cloned().unwrap_or_default();
                    listview::put_text(&mut di.item, &line, &mut self.buf);
                }
                Some(LRESULT(0))
            }
            (id::R_LIST, _) => self.results_notify(nm.code, lp),
            (id::F_TREE | id::F_RECS, _) => self.forensic_notify(cid, nm.code, lp),
            _ => None,
        }
    }

    fn on_ctlcolor(&self, dc: HDC, ctl_: HWND) -> Option<LRESULT> {
        let color = if self.panel.as_ref().is_some_and(|p| p.errors == ctl_) {
            rgb(200, 0, 0)
        } else if ctl_ == self.res.note {
            rgb(176, 96, 0)
        } else {
            return None;
        };
        unsafe {
            SetTextColor(dc, color);
            SetBkMode(dc, TRANSPARENT);
            Some(LRESULT(GetSysColorBrush(COLOR_BTNFACE).0 as isize))
        }
    }

    fn on_drop(&mut self, hdrop: HDROP) {
        let path = unsafe {
            let n = DragQueryFileW(hdrop, 0, None);
            let mut buf = vec![0u16; n as usize + 1];
            let got = DragQueryFileW(hdrop, 0, Some(&mut buf));
            DragFinish(hdrop);
            PathBuf::from(String::from_utf16_lossy(&buf[..got as usize]))
        };
        if self.running() {
            return;
        }
        if path.is_dir() {
            self.set_input(&path.display().to_string());
        } else {
            self.acts.push(Act::Error("폴더를 끌어다 놓으세요.".into()));
        }
    }

    /// 우클릭 메뉴 선택
    fn on_context(&mut self, cmd: u16) {
        if cmd == id::C_EXCL || cmd == id::C_INCL {
            let vals = std::mem::take(&mut self.ctx_values);
            exclude::set_all(&mut self.excluded, &vals, cmd == id::C_EXCL);
            if cmd == id::C_INCL {
                self.drop_formats(&vals);
            }
            self.exclusion_changed();
            return;
        }
        if cmd == id::C_EXCL_FMT {
            let vals = std::mem::take(&mut self.ctx_values);
            self.exclude_formats(&vals);
            self.exclusion_changed();
            return;
        }
        let (Some(scan), Some((s, k))) = (&self.scan, self.ctx_target) else { return };
        match cmd {
            id::C_SRC => self.acts.push(Act::Reveal(scan.path(s).to_path_buf())),
            id::C_DST => {
                if let Some(o) = scan.out_path(s) {
                    self.acts.push(Act::Reveal(o.to_path_buf()));
                }
            }
            id::C_COPY => copy_text(self.hwnd, &scan.records(s)[k].original),
            _ => {}
        }
    }

    /// 파일 분석 탭의 상세 창 높이 조절 막대 끌기
    fn on_splitter(&mut self, msg: u32, pt: POINT) -> bool {
        if self.tab != T_FORENSIC {
            return false;
        }
        match msg {
            WM_LBUTTONDOWN if self.fx_splitter_hit(pt) => {
                self.fx.drag = true;
                unsafe {
                    SetCapture(self.hwnd);
                }
                true
            }
            WM_MOUSEMOVE if self.fx.drag => {
                self.fx_set_split(pt.y);
                true
            }
            WM_LBUTTONUP if self.fx.drag => {
                self.fx.drag = false;
                unsafe {
                    let _ = ReleaseCapture();
                }
                true
            }
            _ => false,
        }
    }

    /// 현재 페이지 영역 (탭 컨트롤 기준)
    fn page(&self) -> R {
        let (w, h) = self.client();
        let mut sr = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.c.status, &mut sr);
        }
        let m = layout::main(w, h, sr.bottom - sr.top, self.dpi());
        self.page_rect(m.tab)
    }
}

fn build_menu() -> HMENU {
    unsafe {
        let bar = CreateMenu().unwrap_or_default();
        let add = |m: HMENU, id: u16, t: &str| {
            let w = ctl::wide(t);
            let _ = AppendMenuW(m, MF_STRING, id as usize, PCWSTR(w.as_ptr()));
        };
        let sub = |title: &str, f: &dyn Fn(HMENU)| {
            let m = CreatePopupMenu().unwrap_or_default();
            f(m);
            let w = ctl::wide(title);
            let _ = AppendMenuW(bar, MF_POPUP, m.0 as usize, PCWSTR(w.as_ptr()));
        };
        sub("파일(&F)", &|m| {
            add(m, id::M_OPEN, "검사할 폴더 열기(&O)...");
            let _ = AppendMenuW(m, MF_SEPARATOR, 0, None);
            add(m, id::M_REPORT, "보고서 열기(&R)");
            add(m, id::M_OUTDIR, "출력 폴더 열기(&D)");
            let _ = AppendMenuW(m, MF_SEPARATOR, 0, None);
            add(m, id::M_EXIT, "끝내기(&X)");
        });
        sub("보기(&V)", &|m| {
            for (i, t) in TABS.iter().enumerate() {
                add(m, id::M_TAB0 + i as u16, t);
            }
            let _ = AppendMenuW(m, MF_SEPARATOR, 0, None);
            add(m, id::M_EXTS, "검사할 확장자 선택(&E)...");
            add(m, id::M_CLEAR_EXCL, "제외 목록 비우기(&C)");
        });
        sub("도움말(&H)", &|m| add(m, id::M_ABOUT, "정보(&A)..."));
        bar
    }
}

/// 클립보드에 글자 복사 (CF_UNICODETEXT)
fn copy_text(hwnd: HWND, s: &str) {
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    let w = ctl::wide(s);
    unsafe {
        if OpenClipboard(hwnd).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(h) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(h);
                // 성공하면 소유권이 클립보드로 넘어감, 실패하면 직접 해제
                if SetClipboardData(13 /* CF_UNICODETEXT */, HANDLE(h.0)).is_err() {
                    let _ = GlobalFree(HGLOBAL(h.0));
                }
            }
        }
        let _ = CloseClipboard();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescan_escalates_then_warns() {
        // 중간 단계에서 남음 → 최고 단계 1번째
        assert_eq!(rescan_next(1, 0, 0, false), Next::Top(1));
        // 최고 단계 1번째에서 남음 → 한 번 더
        assert_eq!(rescan_next(2, 1, 1, false), Next::Top(2));
        // 최고 단계 2번째에서도 남음 → 경고 (그대로든 아니든)
        assert_eq!(rescan_next(2, 2, 2, true), Next::Warn);
        assert_eq!(rescan_next(2, 2, 2, false), Next::Warn);
        // 같은 폴더를 이전에도 최고 단계로 검사했는데 결과가 그대로 → 첫 회라도 경고
        assert_eq!(rescan_next(2, 1, 3, true), Next::Warn);
        assert_eq!(rescan_next(2, 1, 3, false), Next::Top(2));
    }
}

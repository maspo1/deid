//! macOS 화면 — AppKit 네이티브 (objc2). 그리기·배치·스크롤·표 열 크기·트리 펼침은 모두 AppKit 이 한다.
//!
//! 프로세스 구조 (크롬·엣지·웹뷰와 같은 방식)
//! * 화면 프로세스: 창, 설정, 결과 목록만 보유
//! * 엔진 프로세스: 검사할 때만 `deid --engine` 으로 띄우고 끝나면 종료 (`crate::ipc`)
//!
//! 화면 구성 (Process Explorer / NetworkMiner 형식, Windows 화면과 같은 기능)
//! * 메뉴: deid · 파일 · 편집 · 보기 · 윈도우
//! * 위    : 검사 폴더 / 출력 폴더
//! * 왼쪽  : 검출 항목, 탐지 엔진, 그림(OCR), 파일, 사용자 정의 정규식 (`options`)
//! * 가운데: 탭 — 로그 / 검출 결과(`results`) / 파일 분석(`forensic`) / 미리보기(`tabs`)
//! * 아래  : 진행 막대, 시작·중지, 보고서·출력 폴더 열기
//! * 창    : 확장자 선택(`ext_window`), 정보(`about`)
//!
//! 상태는 메인 스레드 전용 `STATE` 하나에 둔다. AppKit 이 데이터 소스를 바로 부를 수 있으므로
//! `STATE` 를 빌린 채로 AppKit 을 부르지 않는다 (값만 꺼내고 놓은 뒤 화면을 바꾼다).

mod about;
mod ext_window;
mod forensic;
mod kit;
mod options;
mod results;
mod tabs;
mod worker;

use crate::gui::scan::Scan;
use crate::gui::settings::Settings;
use crate::gui::sys::{self, Owner};
use crate::exts::ExtFilter;
use kit::{button, field, label, H};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationTerminateReply, NSBackingStoreType, NSButton, NSDragOperation, NSDraggingDestination, NSDraggingInfo, NSImage, NSMenu,
    NSMenuItem, NSOutlineView, NSOutlineViewDataSource, NSOutlineViewDelegate, NSPasteboardTypeFileURL, NSProgressIndicator,
    NSSplitView, NSTabView, NSTabViewDelegate, NSTabViewItem, NSTableColumn, NSTableRowView, NSTableView, NSTableViewDataSource,
    NSTableViewDelegate, NSTextDelegate, NSTextField, NSTextFieldDelegate, NSTextView, NSTextViewDelegate, NSControlTextEditingDelegate,
    NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    NSArray, NSData, NSInteger, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes,
    NSSize, NSString, NSTimer, NSURL,
};
use std::cell::{OnceCell, RefCell};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};
use worker::Handle;

// ───────────────────────────── 상태 ─────────────────────────────

pub struct Ui {
    ctrl: Retained<Controller>,
    window: Retained<NSWindow>,
    input: Retained<NSTextField>,
    output: Retained<NSTextField>,
    browse: [Retained<NSButton>; 2],
    split: Retained<NSSplitView>,
    opts: options::OptionsUi,
    tabs: Retained<NSTabView>,
    log: tabs::LogUi,
    res: results::ResultsUi,
    tree: forensic::TreeUi,
    prev: tabs::PreviewUi,
    progress: Retained<NSProgressIndicator>,
    status: Retained<NSTextField>,
    start: Retained<NSButton>,
    report_btn: Retained<NSButton>,
    output_btn: Retained<NSButton>,
    ext: ext_window::ExtUi,
    about: about::AboutUi,
}

#[derive(Default)]
pub struct State {
    exts: ExtFilter,
    job: Option<Handle>,
    stopping: bool,
    /// 중지가 끝나면 창을 닫는다
    quit_after: bool,
    scan: Option<Scan>,
    /// 로그 보기에 이미 붙인 줄 수
    log_shown: usize,
    /// 표·트리를 마지막으로 다시 만든 때 (검사 중 갱신 간격 조절)
    last_view: Option<Instant>,
    timer: Option<Retained<NSTimer>>,
    res: results::ResState,
    tree: forensic::TreeState,
    prev_recs: Vec<(String, String, String)>,
    ext: ext_window::ExtState,
    about: about::AboutState,
}

thread_local! {
    static UI: OnceCell<&'static Ui> = const { OnceCell::new() };
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn ui() -> &'static Ui {
    UI.with(|u| *u.get().expect("화면이 아직 만들어지지 않음"))
}

/// 화면을 만드는 도중에도 AppKit 이 표 데이터를 물을 수 있다 (그때는 빈 표)
fn try_ui() -> Option<&'static Ui> {
    UI.with(|u| u.get().copied())
}

fn st<R>(f: impl FnOnce(&State) -> R) -> R {
    STATE.with(|s| f(&s.borrow()))
}

fn st_mut<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn running() -> bool {
    st(|s| s.job.is_some())
}

/// 탭 순서
const TAB_LOG: isize = 0;
const TAB_RESULTS: isize = 1;
const TAB_FORENSIC: isize = 2;
const TAB_PREVIEW: isize = 3;

// ───────────────────────────── Objective-C 클래스 ─────────────────────────────

define_class!(
    /// 모든 컨트롤의 대상(target)이자 앱·창·표·트리·글자 입력의 델리게이트
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeidController"]
    pub struct Controller;

    impl Controller {
        // ── 위·아래 ──
        #[unsafe(method(browseInput:))]
        fn browse_input(&self, _s: Option<&AnyObject>) {
            browse(false);
        }
        #[unsafe(method(browseOutput:))]
        fn browse_output(&self, _s: Option<&AnyObject>) {
            browse(true);
        }
        #[unsafe(method(openFolderMenu:))]
        fn open_folder_menu(&self, _s: Option<&AnyObject>) {
            browse(false);
        }
        #[unsafe(method(startStop:))]
        fn start_stop(&self, _s: Option<&AnyObject>) {
            if running() {
                stop();
            } else {
                start();
            }
        }
        #[unsafe(method(openReport:))]
        fn open_report(&self, _s: Option<&AnyObject>) {
            if let Some(p) = finished_path(true) {
                sys::open(&p);
            }
        }
        #[unsafe(method(openOutput:))]
        fn open_output(&self, _s: Option<&AnyObject>) {
            if let Some(p) = finished_path(false) {
                sys::open(&p);
            }
        }
        #[unsafe(method(showTab:))]
        fn show_tab(&self, s: &NSMenuItem) {
            select_tab(s.tag());
        }
        #[unsafe(method(showExt:))]
        fn show_ext(&self, _s: Option<&AnyObject>) {
            ext_window::show();
        }
        #[unsafe(method(showAbout:))]
        fn show_about(&self, _s: Option<&AnyObject>) {
            about::show();
        }
        #[unsafe(method(tick:))]
        fn tick(&self, _s: Option<&AnyObject>) {
            tick();
        }

        // ── 설정 ──
        #[unsafe(method(optionChanged:))]
        fn option_changed(&self, _s: Option<&AnyObject>) {
            options_changed();
        }
        #[unsafe(method(selectAllTypes:))]
        fn select_all_types(&self, _s: Option<&AnyObject>) {
            ui().opts.set_all_types(true);
            options_changed();
        }
        #[unsafe(method(clearAllTypes:))]
        fn clear_all_types(&self, _s: Option<&AnyObject>) {
            ui().opts.set_all_types(false);
            options_changed();
        }
        #[unsafe(method(loadPatterns:))]
        fn load_patterns(&self, _s: Option<&AnyObject>) {
            load_patterns();
        }
        #[unsafe(method(testPatterns:))]
        fn test_patterns(&self, _s: Option<&AnyObject>) {
            select_tab(TAB_PREVIEW);
        }
        #[unsafe(method(resetDefaults:))]
        fn reset_defaults(&self, _s: Option<&AnyObject>) {
            let u = ui();
            let keep = Settings { patterns: kit::tv_text(&u.opts.patterns), ..Default::default() };
            u.opts.write(&keep);
            st_mut(|s| s.exts = keep.exts.clone());
            options_changed();
        }

        // ── 검출 결과 ──
        #[unsafe(method(resultsFilter:))]
        fn results_filter(&self, _s: Option<&AnyObject>) {
            results::refresh();
        }
        #[unsafe(method(resRevealSrc:))]
        fn res_reveal_src(&self, _s: Option<&AnyObject>) {
            results::reveal_src();
        }
        #[unsafe(method(resRevealOut:))]
        fn res_reveal_out(&self, _s: Option<&AnyObject>) {
            results::reveal_out();
        }
        #[unsafe(method(resCopy:))]
        fn res_copy(&self, _s: Option<&AnyObject>) {
            results::copy();
        }

        // ── 파일 분석 ──
        #[unsafe(method(treeFilter:))]
        fn tree_filter(&self, _s: Option<&AnyObject>) {
            forensic::refresh();
        }
        #[unsafe(method(treeExpandAll:))]
        fn tree_expand_all(&self, _s: Option<&AnyObject>) {
            forensic::expand_all(true);
        }
        #[unsafe(method(treeCollapseAll:))]
        fn tree_collapse_all(&self, _s: Option<&AnyObject>) {
            forensic::expand_all(false);
        }
        #[unsafe(method(treeDoubleClick:))]
        fn tree_double_click(&self, _s: Option<&AnyObject>) {
            forensic::double_click();
        }
        #[unsafe(method(detailTab:))]
        fn detail_tab(&self, _s: Option<&AnyObject>) {
            forensic::detail_tab();
        }
        #[unsafe(method(detailCopy:))]
        fn detail_copy(&self, _s: Option<&AnyObject>) {
            forensic::detail_copy();
        }
        #[unsafe(method(revealSrc:))]
        fn reveal_src(&self, _s: Option<&AnyObject>) {
            forensic::reveal(false);
        }
        #[unsafe(method(revealOut:))]
        fn reveal_out(&self, _s: Option<&AnyObject>) {
            forensic::reveal(true);
        }

        // ── 확장자 선택 창 ──
        #[unsafe(method(extFilter:))]
        fn ext_filter(&self, _s: Option<&AnyObject>) {
            ext_window::refresh();
        }
        #[unsafe(method(extSelectShown:))]
        fn ext_select_shown(&self, _s: Option<&AnyObject>) {
            ext_window::select_shown();
        }
        #[unsafe(method(extClearShown:))]
        fn ext_clear_shown(&self, _s: Option<&AnyObject>) {
            ext_window::clear_shown();
        }
        #[unsafe(method(extDefaults:))]
        fn ext_defaults(&self, _s: Option<&AnyObject>) {
            ext_window::defaults();
        }
        #[unsafe(method(extToggle:))]
        fn ext_toggle(&self, s: &NSButton) {
            ext_window::toggle(s);
        }
        #[unsafe(method(extFlags:))]
        fn ext_flags(&self, _s: Option<&AnyObject>) {
            ext_window::flags();
        }

        // ── 정보 창 ──
        #[unsafe(method(aboutFilter:))]
        fn about_filter(&self, _s: Option<&AnyObject>) {
            about::refresh();
        }
        #[unsafe(method(aboutOpen:))]
        fn about_open(&self, _s: Option<&AnyObject>) {
            about::open_clicked();
        }
        #[unsafe(method(openGithub:))]
        fn open_github(&self, _s: Option<&AnyObject>) {
            about::open_github();
        }

        // ── 메뉴 사용 가능 여부 ──
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            let a = item.action();
            if a == Some(sel!(openReport:)) {
                finished_path(true).is_some()
            } else if a == Some(sel!(openOutput:)) {
                finished_path(false).is_some()
            } else if a == Some(sel!(openFolderMenu:)) {
                !running()
            } else {
                true
            }
        }

        // ── 표 (NSTableViewDataSource / Delegate) ──
        #[unsafe(method(numberOfRowsInTableView:))]
        fn rows(&self, tv: &NSTableView) -> NSInteger {
            match which(tv) {
                Some(T::Results) => results::count(),
                Some(T::Recs) => forensic::recs_count(),
                Some(T::Props) => forensic::props_count(),
                Some(T::Preview) => tabs::preview_count(),
                Some(T::Ext) => ext_window::count(),
                Some(T::About) => about::count(),
                None => 0,
            }
        }
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn table_view(&self, tv: &NSTableView, col: Option<&NSTableColumn>, row: NSInteger) -> Option<Retained<NSView>> {
            let r = row as usize;
            match which(tv) {
                Some(T::Results) => results::view_for(tv, col, r),
                Some(T::Recs) => forensic::recs_view(tv, col, r),
                Some(T::Props) => forensic::props_view(tv, col, r),
                Some(T::Preview) => tabs::preview_view(tv, col, r),
                Some(T::Ext) => ext_window::view_for(tv, col, r),
                Some(T::About) => about::view_for(tv, col, r),
                None => None,
            }
        }
        #[unsafe(method(tableView:isGroupRow:))]
        fn is_group_row(&self, tv: &NSTableView, row: NSInteger) -> bool {
            match which(tv) {
                Some(T::Ext) => ext_window::is_group(row as usize),
                Some(T::About) => about::is_group(row as usize),
                _ => false,
            }
        }

        // ── 트리 (NSOutlineViewDataSource / Delegate) ──
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        fn ov_count(&self, _ov: &NSOutlineView, item: Option<&AnyObject>) -> NSInteger {
            forensic::children_count(item)
        }
        #[unsafe(method_id(outlineView:child:ofItem:))]
        fn ov_child(&self, _ov: &NSOutlineView, index: NSInteger, item: Option<&AnyObject>) -> Retained<AnyObject> {
            forensic::child(index, item)
        }
        #[unsafe(method(outlineView:isItemExpandable:))]
        fn ov_expandable(&self, _ov: &NSOutlineView, item: &AnyObject) -> bool {
            forensic::expandable(item)
        }
        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        fn ov_view(&self, ov: &NSOutlineView, col: Option<&NSTableColumn>, item: &AnyObject) -> Option<Retained<NSView>> {
            forensic::view_for(ov, col, item)
        }
        #[unsafe(method(outlineView:didAddRowView:forRow:))]
        fn ov_row(&self, _ov: &NSOutlineView, rv: &NSTableRowView, row: NSInteger) {
            forensic::row_tint(rv, row);
        }
        #[unsafe(method(outlineViewSelectionDidChange:))]
        fn ov_selection(&self, _n: &NSNotification) {
            forensic::selection_changed();
        }
        #[unsafe(method(outlineViewItemDidExpand:))]
        fn ov_expand(&self, n: &NSNotification) {
            forensic::did_expand(notified_item(n).as_deref(), true);
        }
        #[unsafe(method(outlineViewItemDidCollapse:))]
        fn ov_collapse(&self, n: &NSNotification) {
            forensic::did_expand(notified_item(n).as_deref(), false);
        }

        // ── 글자 입력 ──
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, n: &NSNotification) {
            let u = ui();
            let obj = n.object();
            if obj.as_deref().is_some_and(|o| std::ptr::eq(o, (&*u.prev.input as &NSTextView).as_ref())) {
                tabs::preview_refresh();
            } else {
                options_changed();
            }
        }
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _n: &NSNotification) {
            update_output_hint();
        }

        // ── 탭 ──
        #[unsafe(method(tabView:didSelectTabViewItem:))]
        fn tab_selected(&self, _tv: &NSTabView, _item: Option<&NSTabViewItem>) {
            refresh_current_tab();
        }

        // ── 창·앱 ──
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, w: &NSWindow) -> bool {
            !std::ptr::eq(w, &*ui().window) || may_close()
        }
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn terminate_after_close(&self, _a: &NSApplication) -> bool {
            true
        }
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _a: &NSApplication) -> NSApplicationTerminateReply {
            if may_close() {
                NSApplicationTerminateReply::TerminateNow
            } else {
                NSApplicationTerminateReply::TerminateCancel
            }
        }
    }

    unsafe impl NSObjectProtocol for Controller {}
    unsafe impl NSApplicationDelegate for Controller {}
    unsafe impl NSWindowDelegate for Controller {}
    unsafe impl NSTableViewDataSource for Controller {}
    unsafe impl NSTableViewDelegate for Controller {}
    unsafe impl NSOutlineViewDataSource for Controller {}
    unsafe impl NSOutlineViewDelegate for Controller {}
    unsafe impl NSTextDelegate for Controller {}
    unsafe impl NSTextViewDelegate for Controller {}
    unsafe impl NSControlTextEditingDelegate for Controller {}
    unsafe impl NSTextFieldDelegate for Controller {}
    unsafe impl NSTabViewDelegate for Controller {}
);

impl Controller {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    /// 창 내용 — 폴더를 끌어다 놓으면 검사 폴더로
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeidDropView"]
    struct DropView;

    impl DropView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, _s: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            if running() {
                NSDragOperation::None
            } else {
                NSDragOperation::Copy
            }
        }
        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, s: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            dropped(s)
        }
    }

    unsafe impl NSObjectProtocol for DropView {}
    unsafe impl NSDraggingDestination for DropView {}
);

impl DropView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        let v: Retained<Self> = unsafe { msg_send![super(this), init] };
        v.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
        v
    }
}

fn content_of(w: &NSWindow) -> Retained<NSView> {
    w.contentView().expect("창 내용")
}

fn notified_item(n: &NSNotification) -> Option<Retained<AnyObject>> {
    n.userInfo()?.objectForKey(&NSString::from_str("NSObject"))
}

enum T {
    Results,
    Recs,
    Props,
    Preview,
    Ext,
    About,
}

fn which(tv: &NSTableView) -> Option<T> {
    let u = try_ui()?;
    let is = |t: &Retained<NSTableView>| std::ptr::eq(tv, &**t);
    Some(if is(&u.res.table) {
        T::Results
    } else if is(&u.tree.recs) {
        T::Recs
    } else if is(&u.tree.props) {
        T::Props
    } else if is(&u.prev.table) {
        T::Preview
    } else if is(&u.ext.table) {
        T::Ext
    } else if is(&u.about.table) {
        T::About
    } else {
        return None;
    })
}

// ───────────────────────────── 시작 ─────────────────────────────

pub fn run(prefill: Option<PathBuf>) -> ExitCode {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("화면은 메인 스레드에서만 열 수 있습니다");
        return ExitCode::FAILURE;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    // 번들 없이 실행해도 Dock 아이콘이 보이도록 (exe 아이콘과 같은 파일)
    let icon = NSData::with_bytes(include_bytes!("../../../assets/icon/app.ico"));
    if let Some(img) = NSImage::initWithData(NSImage::alloc(), &icon) {
        unsafe { app.setApplicationIconImage(Some(&img)) };
    }
    let ctrl = Controller::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*ctrl)));
    st_mut(|s| s.exts = Settings::default().exts);
    let ui: &'static Ui = Box::leak(Box::new(build(mtm, ctrl)));
    UI.with(|u| u.set(ui).ok());
    menu(mtm, &app);
    if let Some(p) = prefill.filter(|p| p.is_dir()) {
        ui.input.setStringValue(&kit::ns(&p.display().to_string()));
    }
    update_output_hint();
    options_changed();
    set_running(false);
    refresh_current_tab();
    ui.window.center();
    ui.window.makeKeyAndOrderFront(None);
    content_of(&ui.window).layoutSubtreeIfNeeded();
    ui.split.setPosition_ofDividerAtIndex(340.0, 0);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    app.run();
    ExitCode::SUCCESS
}

fn build(mtm: MainThreadMarker, ctrl: Retained<Controller>) -> Ui {
    let t: &AnyObject = ctrl.as_ref();
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable | NSWindowStyleMask::Miniaturizable;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1100.0, 740.0));
    let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), rect, style, NSBackingStoreType::Buffered, false) };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&kit::ns(sys::APP_TITLE));
    window.setContentMinSize(NSSize::new(820.0, 540.0));
    window.setDelegate(Some(ProtocolObject::from_ref(&*ctrl)));
    let content = DropView::new(mtm);
    window.setContentView(Some(&content));

    // 위: 경로
    let input = field("", "폴더를 선택하거나 창에 끌어다 놓으세요");
    unsafe { input.setDelegate(Some(ProtocolObject::from_ref(&*ctrl))) };
    let output = field("", "");
    let browse = [button("찾아보기...", t, sel!(browseInput:)), button("찾아보기...", t, sel!(browseOutput:))];
    let path_row = |name: &str, f: &NSTextField, b: &NSButton| {
        let l = label(name);
        kit::width(&l, 64.0);
        kit::stretch(f);
        kit::hstack(&[&l, f, b])
    };
    let in_row = path_row("검사 폴더", &input, &browse[0]);
    let out_row = path_row("출력 폴더", &output, &browse[1]);

    // 가운데: 설정 | 탭
    let opts = options::OptionsUi::build(&ctrl);
    let log = tabs::LogUi::build();
    let res = results::ResultsUi::build(&ctrl);
    let tree = forensic::TreeUi::build(&ctrl);
    let prev = tabs::PreviewUi::build(&ctrl);
    let tabs_view = NSTabView::new(mtm);
    for (label, view) in [("로그", &*log.scroll as &NSView), ("검출 결과", &res.view), ("파일 분석", &tree.view), ("미리보기", &prev.view)] {
        let item = NSTabViewItem::new();
        item.setLabel(&kit::ns(label));
        item.setView(Some(view));
        tabs_view.addTabViewItem(&item);
    }
    tabs_view.setDelegate(Some(ProtocolObject::from_ref(&*ctrl)));
    let split = NSSplitView::new(mtm);
    split.setVertical(true);
    split.setDividerStyle(objc2_app_kit::NSSplitViewDividerStyle::Thin);
    split.addArrangedSubview(&opts.scroll);
    split.addArrangedSubview(&tabs_view);
    opts.scroll.widthAnchor().constraintGreaterThanOrEqualToConstant(290.0).setActive(true);
    opts.scroll.widthAnchor().constraintLessThanOrEqualToConstant(540.0).setActive(true);
    tabs_view.widthAnchor().constraintGreaterThanOrEqualToConstant(420.0).setActive(true);
    split.setHoldingPriority_forSubviewAtIndex(260.0, 0);
    split.setHoldingPriority_forSubviewAtIndex(250.0, 1);

    // 아래: 진행
    let progress = NSProgressIndicator::new(mtm);
    progress.setIndeterminate(false);
    progress.setMinValue(0.0);
    progress.setMaxValue(1.0);
    let status = label("준비");
    status.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
    kit::stretch(&status);
    let report_btn = button("보고서 열기", t, sel!(openReport:));
    let output_btn = button("출력 폴더 열기", t, sel!(openOutput:));
    let start = button("검사 시작", t, sel!(startStop:));
    start.setKeyEquivalent(&kit::ns("\r"));
    kit::width(&start, 110.0);
    let bottom = kit::hstack(&[&status, &report_btn, &output_btn, &start]);

    kit::column(&content, &[(&in_row, H::Auto), (&out_row, H::Auto), (&split, H::Fill), (&progress, H::Auto), (&bottom, H::Auto)], 12.0, 8.0);

    let ext = ext_window::ExtUi::build(&ctrl);
    let about = about::AboutUi::build(&ctrl);
    Ui { ctrl, window, input, output, browse, split, opts, tabs: tabs_view, log, res, tree, prev, progress, status, start, report_btn, output_btn, ext, about }
}

fn menu_item(title: &str, action: Option<Sel>, key: &str, target: bool) -> Retained<NSMenuItem> {
    let m = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(kit::mtm()), &kit::ns(title), action, &kit::ns(key)) };
    if target {
        unsafe { m.setTarget(Some(ui().ctrl.as_ref())) };
    }
    m
}

/// 메뉴 막대. target 이 없는 항목(복사·붙여넣기 등)은 응답자 체인이 처리한다.
fn menu(mtm: MainThreadMarker, app: &NSApplication) {
    let bar = NSMenu::new(mtm);
    let sub = |title: &str, items: Vec<Retained<NSMenuItem>>| {
        let m = NSMenu::initWithTitle(NSMenu::alloc(mtm), &kit::ns(title));
        for i in items {
            m.addItem(&i);
        }
        let top = NSMenuItem::new(mtm);
        top.setSubmenu(Some(&m));
        bar.addItem(&top);
    };
    let sep = || NSMenuItem::separatorItem(mtm);
    sub(
        "deid",
        vec![
            menu_item("deid 정보", Some(sel!(showAbout:)), "", true),
            sep(),
            menu_item("deid 가리기", Some(sel!(hide:)), "h", false),
            menu_item("deid 종료", Some(sel!(terminate:)), "q", false),
        ],
    );
    sub(
        "파일",
        vec![
            menu_item("검사할 폴더 열기...", Some(sel!(openFolderMenu:)), "o", true),
            sep(),
            menu_item("보고서 열기", Some(sel!(openReport:)), "", true),
            menu_item("출력 폴더 열기", Some(sel!(openOutput:)), "", true),
            sep(),
            menu_item("창 닫기", Some(sel!(performClose:)), "w", false),
        ],
    );
    sub(
        "편집",
        vec![
            menu_item("실행 취소", Some(sel!(undo:)), "z", false),
            menu_item("실행 복귀", Some(sel!(redo:)), "Z", false),
            sep(),
            menu_item("오려두기", Some(sel!(cut:)), "x", false),
            menu_item("복사하기", Some(sel!(copy:)), "c", false),
            menu_item("붙여넣기", Some(sel!(paste:)), "v", false),
            menu_item("전체 선택", Some(sel!(selectAll:)), "a", false),
        ],
    );
    let mut view: Vec<Retained<NSMenuItem>> = ["로그", "검출 결과", "파일 분석", "미리보기"]
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let m = menu_item(t, Some(sel!(showTab:)), &(i + 1).to_string(), true);
            m.setTag(i as isize);
            m
        })
        .collect();
    view.push(sep());
    view.push(menu_item("검사할 확장자 선택...", Some(sel!(showExt:)), "", true));
    sub("보기", view);
    sub("윈도우", vec![menu_item("최소화", Some(sel!(performMiniaturize:)), "m", false)]);
    app.setMainMenu(Some(&bar));
}

// ───────────────────────────── 동작 ─────────────────────────────

fn settings() -> Settings {
    let u = ui();
    st(|s| u.opts.read(&s.exts, kit::text(&u.input), kit::text(&u.output)))
}

fn alert(msg: &str) {
    let a = NSAlert::new(kit::mtm());
    a.setMessageText(&kit::ns("알림"));
    a.setInformativeText(&kit::ns(msg));
    a.beginSheetModalForWindow_completionHandler(&ui().window, None);
}

fn browse(output: bool) {
    if running() {
        return;
    }
    let title = if output { "출력 폴더 선택 (검사할 폴더 바깥)" } else { "검사할 폴더 선택" };
    if let Some(p) = sys::pick_folder(Owner::default(), title) {
        let f = if output { &ui().output } else { &ui().input };
        f.setStringValue(&kit::ns(&p.display().to_string()));
        update_output_hint();
    }
}

fn dropped(info: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
    if running() {
        return false;
    }
    let pb = info.draggingPasteboard();
    let Some(url) = pb.stringForType(unsafe { NSPasteboardTypeFileURL }) else { return false };
    let Some(path) = NSURL::URLWithString(&url).and_then(|u| u.path()) else { return false };
    let p = PathBuf::from(path.to_string());
    if p.is_dir() {
        ui().input.setStringValue(&kit::ns(&p.display().to_string()));
        update_output_hint();
        true
    } else {
        alert("폴더를 끌어다 놓으세요.");
        false
    }
}

fn update_output_hint() {
    let u = ui();
    let hint = match kit::text(&u.input).trim().trim_matches('"') {
        "" => "비워 두면 <검사 폴더>_검사완료".to_string(),
        i => format!("비워 두면 {}", crate::pipeline::default_output(std::path::Path::new(i)).display()),
    };
    u.output.setPlaceholderString(Some(&kit::ns(&hint)));
}

fn load_patterns() {
    let filters: [(&str, &[&str]); 2] = [("패턴 파일", &["txt", "pat", "cfg"]), ("모든 파일", &["*"])];
    let Some(p) = sys::pick_file(Owner::default(), "사용자 정의 정규식 파일 (줄마다 라벨=정규식)", &filters) else { return };
    match std::fs::read(&p) {
        Ok(b) => {
            let t = crate::common::decode_text(&b).map(|(t, _)| t).unwrap_or_else(|| String::from_utf8_lossy(&b).into_owned());
            let tv = &ui().opts.patterns;
            let cur = kit::tv_text(tv);
            let cur = cur.trim_end();
            tv.setString(&kit::ns(&if cur.is_empty() { t } else { format!("{cur}\n{t}") }));
            options_changed();
        }
        Err(e) => alert(&format!("파일을 읽을 수 없습니다: {e}")),
    }
}

/// 설정이 바뀜 → 딸린 표시, 미리보기
fn options_changed() {
    let u = ui();
    let exts = st(|s| s.exts.clone());
    u.opts.sync(&exts, !running());
    if u.tabs.indexOfTabViewItem(&u.tabs.selectedTabViewItem().expect("탭")) == TAB_PREVIEW {
        tabs::preview_refresh();
    }
}

fn select_tab(i: isize) {
    ui().tabs.selectTabViewItemAtIndex(i);
}

fn current_tab() -> isize {
    let t = &ui().tabs;
    t.selectedTabViewItem().map_or(0, |i| t.indexOfTabViewItem(&i))
}

fn refresh_current_tab() {
    match current_tab() {
        TAB_RESULTS => results::refresh(),
        TAB_FORENSIC => {
            ui().tree.place_divider();
            forensic::refresh();
        }
        TAB_PREVIEW => tabs::preview_refresh(),
        _ => {}
    }
}

fn finished_path(report: bool) -> Option<PathBuf> {
    st(|s| {
        if s.job.is_some() {
            return None;
        }
        let f = s.scan.as_ref()?.finished.as_ref()?;
        if report {
            f.report.clone()
        } else {
            f.output_dir.clone()
        }
    })
}

fn start() {
    let s = settings();
    let job = match s.job() {
        Ok(j) => j,
        Err(e) => return alert(&e),
    };
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
    let h = match worker::spawn(&job) {
        Ok(h) => h,
        Err(e) => return alert(&e),
    };
    forensic::reset();
    let timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(0.1, ui().ctrl.as_ref(), sel!(tick:), None, true) };
    // 창 크기 조절·메뉴 사용 중에도 진행 표시가 멈추지 않도록
    unsafe { NSRunLoop::currentRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    st_mut(|s| {
        s.job = Some(h);
        s.stopping = false;
        s.scan = Some(scan);
        s.log_shown = 0;
        s.last_view = None;
        s.timer = Some(timer);
    });
    ui().log.clear();
    if current_tab() == TAB_PREVIEW {
        select_tab(TAB_LOG);
    }
    set_running(true);
    tick();
}

fn stop() {
    st_mut(|s| {
        if let Some(h) = &mut s.job {
            h.stop();
            s.stopping = true;
        }
    });
    update_status();
}

/// 검사 중이면 확인 후 중지 (중지가 끝나면 닫힘). 닫아도 되면 true.
fn may_close() -> bool {
    let (running, stopping) = st(|s| (s.job.is_some(), s.stopping));
    if !running {
        return true;
    }
    if !stopping {
        let a = NSAlert::new(kit::mtm());
        a.setAlertStyle(NSAlertStyle::Warning);
        a.setMessageText(&kit::ns("검사 중입니다. 중지하고 닫을까요?"));
        a.setInformativeText(&kit::ns("출력 폴더는 불완전한 상태로 남습니다."));
        a.addButtonWithTitle(&kit::ns("중지하고 닫기"));
        a.addButtonWithTitle(&kit::ns("계속 검사"));
        if a.runModal() != NSAlertFirstButtonReturn {
            return false;
        }
        // 대화상자가 떠 있는 동안 검사가 끝났으면 바로 닫는다
        if st(|s| s.job.is_none()) {
            return true;
        }
        stop();
    }
    st_mut(|s| s.quit_after = true);
    false
}

/// 검사 중 화면 잠금
fn set_running(on: bool) {
    let u = ui();
    u.input.setEnabled(!on);
    u.output.setEnabled(!on);
    for b in &u.browse {
        b.setEnabled(!on);
    }
    u.start.setTitle(&kit::ns(if on { "중지" } else { "검사 시작" }));
    u.start.setBezelColor(on.then(objc2_app_kit::NSColor::systemRedColor).as_deref());
    let done = !on && finished_path(true).is_some();
    u.report_btn.setEnabled(done);
    u.output_btn.setEnabled(!on && finished_path(false).is_some());
    options_changed();
    if u.ext.window.isVisible() {
        ext_window::refresh();
    }
    update_status();
}

/// 진행 막대·상태 문구·탭 제목
fn update_status() {
    let u = ui();
    let (frac, preparing, status, counts) = st(|s| {
        let (frac, preparing) = match &s.scan {
            Some(x) if x.running() => match x.total {
                Some(t) if t > 0 => (x.done as f64 / t as f64, false),
                _ => (0.0, true),
            },
            Some(x) if x.finished.as_ref().is_some_and(|f| f.ok) => (1.0, false),
            _ => (0.0, false),
        };
        let status = match &s.scan {
            _ if s.stopping => "중지하는 중... (처리 중인 파일이 끝나면 멈춥니다)".to_string(),
            Some(x) => x.status_line(),
            None => "준비".into(),
        };
        let counts = s.scan.as_ref().map_or((0, 0, 0), |x| (x.all_records().count(), x.all_records().filter(|r| r.review).count(), x.entries.len()));
        (frac, preparing, status, counts)
    });
    u.progress.setIndeterminate(preparing);
    unsafe {
        if preparing {
            u.progress.startAnimation(None);
        } else {
            u.progress.stopAnimation(None);
        }
    }
    u.progress.setDoubleValue(frac);
    u.status.setStringValue(&kit::ns(&status));
    u.start.setEnabled(!st(|s| s.stopping));
    let (n, review, files) = counts;
    let res = if review > 0 { format!("검출 결과 ({n}, 검토 {review})") } else { format!("검출 결과 ({n})") };
    for (i, t) in [(TAB_RESULTS, res), (TAB_FORENSIC, format!("파일 분석 ({files})"))] {
        u.tabs.tabViewItemAtIndex(i).setLabel(&kit::ns(&t));
    }
}

/// 타이머: 엔진 메시지 반영
fn tick() {
    let (finished, failed, new_lines, refresh_views) = st_mut(|s| {
        let (Some(h), Some(scan)) = (&s.job, &mut s.scan) else { return (false, None, vec![], false) };
        let mut finished = false;
        while let Ok(ev) = h.rx.try_recv() {
            finished |= scan.apply(ev);
        }
        let new_lines = scan.log[s.log_shown..].to_vec();
        s.log_shown = scan.log.len();
        let mut failed = None;
        if finished {
            s.job = None; // 엔진 프로세스 정리
            s.stopping = false;
            if let Some(t) = s.timer.take() {
                t.invalidate();
            }
            if scan.finished.as_ref().is_some_and(|f| !f.ok) {
                failed = scan.log.last().cloned();
            }
        }
        // 검사 중 표·트리는 0.5초마다
        let due = s.last_view.map_or(true, |t| t.elapsed() >= Duration::from_millis(500));
        if due || finished {
            s.last_view = Some(Instant::now());
        }
        (finished, failed, new_lines, due || finished)
    });
    ui().log.append(&new_lines);
    if refresh_views {
        refresh_current_tab();
        update_status();
    }
    if !finished {
        return;
    }
    set_running(false);
    if st(|s| s.quit_after) {
        ui().window.close();
        NSApplication::sharedApplication(kit::mtm()).terminate(None);
        return;
    }
    match failed {
        Some(e) => alert(&e),
        None => {
            let any = st(|s| s.scan.as_ref().is_some_and(|x| x.all_records().next().is_some()));
            if any && current_tab() == TAB_LOG {
                select_tab(TAB_RESULTS);
            }
        }
    }
}

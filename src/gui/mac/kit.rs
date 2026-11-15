//! AppKit 위젯 도우미 — 만들기, 배치(Auto Layout), 표 셀.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSButton, NSColor, NSFont, NSLayoutAttribute, NSLayoutConstraintOrientation, NSLineBreakMode, NSScrollView, NSSearchField,
    NSStackView, NSStackViewDistribution, NSTableCellView, NSTableColumn, NSTableView, NSTableViewColumnAutoresizingStyle,
    NSTableViewStyle, NSTextAlignment, NSTextField, NSTextView, NSUserInterfaceItemIdentification, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSArray, NSString};

pub fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

pub fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("화면은 메인 스레드에서만 다룬다")
}

// ───────────────────────────── 글꼴·색 ─────────────────────────────

pub fn font(size: f64, bold: bool) -> Retained<NSFont> {
    if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    }
}

pub fn mono(size: f64) -> Retained<NSFont> {
    NSFont::monospacedSystemFontOfSize_weight(size, 0.0)
}

/// 상태 색 (밝은·어두운 모드 모두 시스템 색 + 투명도로 맞춘다)
pub fn tint(c: Retained<NSColor>, alpha: f64) -> Retained<NSColor> {
    c.colorWithAlphaComponent(alpha)
}

pub fn warn_color() -> Retained<NSColor> {
    NSColor::systemOrangeColor()
}

// ───────────────────────────── 컨트롤 ─────────────────────────────

pub fn label(text: &str) -> Retained<NSTextField> {
    NSTextField::labelWithString(&ns(text), mtm())
}

pub fn bold_label(text: &str) -> Retained<NSTextField> {
    let l = label(text);
    l.setFont(Some(&font(13.0, true)));
    l
}

pub fn small_label(text: &str) -> Retained<NSTextField> {
    let l = label(text);
    l.setFont(Some(&font(11.0, false)));
    l.setTextColor(Some(&NSColor::secondaryLabelColor()));
    l
}

/// 여러 줄로 감기는 안내 문구
pub fn note(text: &str) -> Retained<NSTextField> {
    let l = NSTextField::wrappingLabelWithString(&ns(text), mtm());
    l.setFont(Some(&font(11.0, false)));
    l.setTextColor(Some(&NSColor::secondaryLabelColor()));
    l.setSelectable(false);
    l
}

pub fn button(title: &str, target: &AnyObject, action: Sel) -> Retained<NSButton> {
    unsafe { NSButton::buttonWithTitle_target_action(&ns(title), Some(target), Some(action), mtm()) }
}

pub fn checkbox(title: &str, target: &AnyObject, action: Sel) -> Retained<NSButton> {
    unsafe { NSButton::checkboxWithTitle_target_action(&ns(title), Some(target), Some(action), mtm()) }
}

pub fn is_on(b: &NSButton) -> bool {
    b.state() != 0
}

pub fn set_on(b: &NSButton, on: bool) {
    b.setState(on as isize);
}

pub fn field(text: &str, placeholder: &str) -> Retained<NSTextField> {
    let f = NSTextField::textFieldWithString(&ns(text), mtm());
    f.setPlaceholderString(Some(&ns(placeholder)));
    f
}

pub fn search(placeholder: &str, target: &AnyObject, action: Sel) -> Retained<NSSearchField> {
    let s = NSSearchField::new(mtm());
    s.setPlaceholderString(Some(&ns(placeholder)));
    unsafe {
        s.setTarget(Some(target));
        s.setAction(Some(action));
    }
    width(&s, 200.0);
    s
}

pub fn text(f: &NSTextField) -> String {
    f.stringValue().to_string()
}

/// 글자 입력 칸 (스크롤). 정규식·문장 입력에 방해되는 자동 고침을 모두 끈다.
pub fn text_view(editable: bool) -> (Retained<NSScrollView>, Retained<NSTextView>) {
    let scroll = NSTextView::scrollableTextView(mtm());
    let tv = scroll.documentView().and_then(|v| v.downcast::<NSTextView>().ok()).expect("scrollableTextView");
    tv.setFont(Some(&mono(12.0)));
    tv.setEditable(editable);
    tv.setRichText(false);
    tv.setAutomaticQuoteSubstitutionEnabled(false);
    tv.setAutomaticDashSubstitutionEnabled(false);
    tv.setAutomaticTextReplacementEnabled(false);
    tv.setAutomaticSpellingCorrectionEnabled(false);
    tv.setContinuousSpellCheckingEnabled(false);
    tv.setAutomaticLinkDetectionEnabled(false);
    scroll.setBorderType(objc2_app_kit::NSBorderType::BezelBorder);
    (scroll, tv)
}

pub fn tv_text(tv: &NSTextView) -> String {
    tv.string().to_string()
}

// ───────────────────────────── 배치 ─────────────────────────────

pub fn stack(views: &[&NSView], vertical: bool, spacing: f64) -> Retained<NSStackView> {
    let s = NSStackView::stackViewWithViews(&NSArray::from_slice(views), mtm());
    if vertical {
        s.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        s.setAlignment(NSLayoutAttribute::Leading);
    } else {
        s.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        s.setAlignment(NSLayoutAttribute::CenterY);
    }
    s.setSpacing(spacing);
    s.setDistribution(NSStackViewDistribution::Fill);
    s
}

pub fn hstack(views: &[&NSView]) -> Retained<NSStackView> {
    stack(views, false, 8.0)
}

pub fn vstack(views: &[&NSView]) -> Retained<NSStackView> {
    stack(views, true, 6.0)
}

/// 남는 가로 공간을 차지하는 빈 칸 (가로 스택에서 오른쪽 정렬용)
pub fn spacer() -> Retained<NSView> {
    let v = NSView::new(mtm());
    stretch(&v);
    v
}

/// 가로로 늘어나고 줄어드는 뷰 (입력 칸·상태 문구)
pub fn stretch(v: &NSView) {
    v.setContentHuggingPriority_forOrientation(1.0, NSLayoutConstraintOrientation::Horizontal);
    v.setContentCompressionResistancePriority_forOrientation(250.0, NSLayoutConstraintOrientation::Horizontal);
}

pub fn width(v: &NSView, w: f64) {
    v.widthAnchor().constraintEqualToConstant(w).setActive(true);
}

pub fn height(v: &NSView, h: f64) {
    v.heightAnchor().constraintEqualToConstant(h).setActive(true);
}

/// 세로 배치 높이 규칙
pub enum H {
    /// 내용 크기
    Auto,
    Fixed(f64),
    /// 남는 공간 전부 (하나만)
    Fill,
}

/// `parent` 안에 위에서 아래로 쌓는다. 모든 항목은 좌우 폭을 꽉 채운다.
pub fn column(parent: &NSView, items: &[(&NSView, H)], pad: f64, gap: f64) {
    let mut prev: Option<&NSView> = None;
    for (v, h) in items {
        v.setTranslatesAutoresizingMaskIntoConstraints(false);
        parent.addSubview(v);
        v.leadingAnchor().constraintEqualToAnchor_constant(&parent.leadingAnchor(), pad).setActive(true);
        v.trailingAnchor().constraintEqualToAnchor_constant(&parent.trailingAnchor(), -pad).setActive(true);
        match prev {
            None => v.topAnchor().constraintEqualToAnchor_constant(&parent.topAnchor(), pad).setActive(true),
            Some(p) => v.topAnchor().constraintEqualToAnchor_constant(&p.bottomAnchor(), gap).setActive(true),
        }
        match h {
            H::Auto => {
                v.setContentHuggingPriority_forOrientation(750.0, NSLayoutConstraintOrientation::Vertical);
                // 스택은 일반 hugging 이 아니라 스택 전용 값으로 늘어남을 막는다 (기본값은 낮아서 남는 높이를 가져간다)
                if let Some(s) = v.downcast_ref::<NSStackView>() {
                    s.setHuggingPriority_forOrientation(750.0, NSLayoutConstraintOrientation::Vertical);
                }
            }
            H::Fixed(x) => height(v, *x),
            H::Fill => v.setContentHuggingPriority_forOrientation(1.0, NSLayoutConstraintOrientation::Vertical),
        }
        prev = Some(v);
    }
    if let Some(p) = prev {
        p.bottomAnchor().constraintEqualToAnchor_constant(&parent.bottomAnchor(), -pad).setActive(true);
    }
}

/// `v` 를 `parent` 에 꽉 채운다
pub fn fill(parent: &NSView, v: &NSView) {
    column(parent, &[(v, H::Fill)], 0.0, 0.0);
}

/// 빈 컨테이너
pub fn container() -> Retained<NSView> {
    NSView::new(mtm())
}

define_class!(
    /// 위에서부터 내용을 쌓는 스크롤 문서 뷰 (AppKit 기본은 아래쪽 원점)
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DeidFlippedView"]
    pub struct FlippedView;

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedView {
    pub fn new() -> Retained<Self> {
        let this = Self::alloc(mtm()).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// 세로 스크롤되는 패널 (내용 폭은 스크롤 폭에 맞춘다)
pub fn scroll_panel(content: &NSView, pad: f64) -> Retained<NSScrollView> {
    let scroll = NSScrollView::new(mtm());
    scroll.setHasVerticalScroller(true);
    scroll.setAutohidesScrollers(true);
    scroll.setDrawsBackground(false);
    let doc = FlippedView::new();
    doc.setTranslatesAutoresizingMaskIntoConstraints(false);
    column(&doc, &[(content, H::Auto)], pad, 0.0);
    scroll.setDocumentView(Some(&doc));
    let clip = scroll.contentView();
    doc.leadingAnchor().constraintEqualToAnchor(&clip.leadingAnchor()).setActive(true);
    doc.topAnchor().constraintEqualToAnchor(&clip.topAnchor()).setActive(true);
    doc.widthAnchor().constraintEqualToAnchor(&clip.widthAnchor()).setActive(true);
    scroll
}

// ───────────────────────────── 표 ─────────────────────────────

/// 열 정의: (식별자, 제목, 폭)
pub type Col<'a> = (&'a str, &'a str, f64);

fn add_columns(tv: &NSTableView, cols: &[Col]) {
    for (id, title, w) in cols {
        let c = NSTableColumn::initWithIdentifier(NSTableColumn::alloc(mtm()), &ns(id));
        c.setTitle(&ns(title));
        c.setWidth(*w);
        c.setMinWidth(36.0);
        tv.addTableColumn(&c);
    }
    tv.setStyle(NSTableViewStyle::FullWidth);
    tv.setColumnAutoresizingStyle(NSTableViewColumnAutoresizingStyle::LastColumnOnlyAutoresizingStyle);
    tv.setAllowsMultipleSelection(false);
}

pub fn scroll_of(doc: &NSView) -> Retained<NSScrollView> {
    let s = NSScrollView::new(mtm());
    s.setHasVerticalScroller(true);
    s.setHasHorizontalScroller(true);
    s.setAutohidesScrollers(true);
    s.setBorderType(objc2_app_kit::NSBorderType::BezelBorder);
    s.setDocumentView(Some(doc));
    s
}

pub fn table(cols: &[Col]) -> (Retained<NSScrollView>, Retained<NSTableView>) {
    let tv = NSTableView::new(mtm());
    add_columns(&tv, cols);
    tv.setUsesAlternatingRowBackgroundColors(true);
    (scroll_of(&tv), tv)
}

pub fn outline(cols: &[Col]) -> (Retained<NSScrollView>, Retained<objc2_app_kit::NSOutlineView>) {
    let ov = objc2_app_kit::NSOutlineView::new(mtm());
    add_columns(&ov, cols);
    unsafe { ov.setOutlineTableColumn(ov.tableColumns().firstObject().as_deref()) };
    ov.setIndentationPerLevel(14.0);
    (scroll_of(&ov), ov)
}

/// 셀 모양
#[derive(Clone, Copy, Default)]
pub struct Look {
    pub mono: bool,
    pub bold: bool,
    pub dim: bool,
    pub warn: bool,
    pub right: bool,
}

pub const PLAIN: Look = Look { mono: false, bold: false, dim: false, warn: false, right: false };

/// 표 셀 (재사용). `badge` 는 글자 뒤에 주황색으로 붙는 짧은 표시.
pub fn cell(tv: &NSTableView, col: Option<&NSTableColumn>, text: &str, look: Look, badge: Option<&str>) -> Retained<NSView> {
    let id = col.map(|c| c.identifier()).unwrap_or_else(|| ns("group"));
    let reused = unsafe { tv.makeViewWithIdentifier_owner(&id, None) }.and_then(|v| v.downcast::<NSTableCellView>().ok());
    let cv = reused.unwrap_or_else(|| {
        let cv = NSTableCellView::new(mtm());
        cv.setIdentifier(Some(&id));
        let tf = label("");
        tf.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        tf.setMaximumNumberOfLines(1);
        tf.setTranslatesAutoresizingMaskIntoConstraints(false);
        stretch(&tf);
        cv.addSubview(&tf);
        tf.leadingAnchor().constraintEqualToAnchor_constant(&cv.leadingAnchor(), 2.0).setActive(true);
        tf.trailingAnchor().constraintEqualToAnchor_constant(&cv.trailingAnchor(), -2.0).setActive(true);
        tf.centerYAnchor().constraintEqualToAnchor(&cv.centerYAnchor()).setActive(true);
        unsafe { cv.setTextField(Some(&tf)) };
        cv
    });
    let tf = unsafe { cv.textField() }.expect("셀 글자");
    let f = if look.mono { mono(12.0) } else { font(13.0, look.bold) };
    let color = if look.warn {
        warn_color()
    } else if look.dim {
        NSColor::secondaryLabelColor()
    } else {
        NSColor::labelColor()
    };
    tf.setFont(Some(&f));
    tf.setTextColor(Some(&color));
    tf.setAlignment(if look.right { NSTextAlignment::Right } else { NSTextAlignment::Natural });
    match badge {
        None => tf.setStringValue(&ns(text)),
        Some(b) => tf.setAttributedStringValue(&with_badge(text, b, &f, &color)),
    }
    tf.setToolTip((!text.is_empty()).then(|| ns(text)).as_deref());
    cv.into_super()
}

fn with_badge(text: &str, badge: &str, f: &NSFont, color: &NSColor) -> Retained<objc2_foundation::NSAttributedString> {
    use objc2_app_kit::{NSFontAttributeName, NSForegroundColorAttributeName};
    use objc2_foundation::{NSMutableAttributedString, NSRange};
    let full = format!("{text}  {badge}");
    let s = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &ns(&full));
    let all = NSRange::new(0, full.encode_utf16().count());
    let tail = NSRange::new(text.encode_utf16().count() + 2, badge.encode_utf16().count());
    let small = font(11.0, true);
    unsafe {
        s.addAttribute_value_range(NSFontAttributeName, f, all);
        s.addAttribute_value_range(NSForegroundColorAttributeName, color, all);
        s.addAttribute_value_range(NSForegroundColorAttributeName, &warn_color(), tail);
        s.addAttribute_value_range(NSFontAttributeName, &small, tail);
    }
    s.into_super()
}

/// 표 열 식별자
pub fn col_id(col: Option<&NSTableColumn>) -> String {
    col.map(|c| c.identifier().to_string()).unwrap_or_default()
}

/// 표의 행 번호 → usize (선택 없음은 None)
pub fn row(n: isize) -> Option<usize> {
    (n >= 0).then_some(n as usize)
}

pub fn copy_text(t: &str) {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    pb.setString_forType(&ns(t), unsafe { NSPasteboardTypeString });
}

pub fn human(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

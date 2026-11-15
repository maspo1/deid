//! 파일 분석 (포렌식 보기) — Process Explorer 의 트리 표 + NetworkMiner 의 상세 창 구조.
//!
//! * 위: 폴더 트리 (NSOutlineView). 검사한 모든 파일의 형식(매직바이트), 크기, 검출·검토 건수, 상태를 색으로 구분.
//!   파일을 펼치면 정보 타입별 묶음 → 값으로 내려간다
//! * 아래: 선택한 파일(폴더)의 상세 — 검출 정보 목록 / 파일 정보
//!
//! 펼침 상태는 노드 키(경로)로 기억해 검사 중 트리를 다시 만들어도 유지한다.
//! 기본: 폴더·타입 묶음은 펼침, 파일은 접힘.

use super::kit::{self, button, cell, checkbox, col_id, is_on, label, note, tint, Look, H, PLAIN};
use super::{st, st_mut, ui, Controller};
use crate::gui::scan::{Scan, Src};
use crate::gui::sys;
use crate::ipc::{Rec, Status};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{sel, MainThreadOnly};
use objc2_app_kit::{
    NSButton, NSColor, NSMenu, NSMenuItem, NSOutlineView, NSScrollView, NSSearchField, NSSegmentSwitchTracking, NSSegmentedControl,
    NSSplitView, NSTableColumn, NSTableRowView, NSTableView, NSTextField, NSView,
};
use objc2_foundation::{NSArray, NSIndexSet, NSNumber};
use std::collections::{BTreeMap, HashMap};

// ───────────────────────────── 모델 ─────────────────────────────

#[derive(Default, Clone, Copy)]
struct Agg {
    files: usize,
    recs: usize,
    review: usize,
    masked: usize,
    size: u64,
}

impl Agg {
    fn add(&mut self, o: &Agg) {
        self.files += o.files;
        self.recs += o.recs;
        self.review += o.review;
        self.masked += o.masked;
        self.size += o.size;
    }
}

#[derive(Clone)]
enum NodeKind {
    /// 상대 경로
    Dir(String),
    File(usize),
    /// 펼친 파일 안의 정보 타입 묶음 (파일 번호)
    Type(usize),
    /// 묶음 안의 값 1건 (파일 번호, 검출 위치, 번호)
    Item(usize, Src, usize),
}

struct Node {
    name: String,
    kind: NodeKind,
    agg: Agg,
    /// 이름에서 나온 검출 (names 의 번호)
    name_hit: Option<usize>,
    children: Vec<usize>,
    /// 펼침 상태 키
    key: String,
}

impl Node {
    fn default_open(&self) -> bool {
        !matches!(self.kind, NodeKind::File(_))
    }
}

#[derive(Clone, PartialEq)]
enum Sel {
    File(usize),
    Dir(String),
}

#[derive(Default)]
pub struct TreeState {
    nodes: Vec<Node>,
    roots: Vec<usize>,
    /// 노드 번호를 담은 NSOutlineView 항목 (트리를 다시 만들 때까지 유지해야 한다)
    items: Vec<Retained<NSNumber>>,
    /// 키 → 펼침 (없으면 기본값)
    open: HashMap<String, bool>,
    built: Option<(u64, bool, String)>,
    /// 프로그램이 펼침·선택을 바꾸는 중 (알림 무시)
    quiet: bool,
    sel: Option<Sel>,
    detail: Vec<(Src, usize)>,
    detail_path: bool,
    props: Vec<(String, String)>,
}

impl TreeState {
    fn is_open(&self, i: usize) -> bool {
        let n = &self.nodes[i];
        self.open.get(&n.key).copied().unwrap_or_else(|| n.default_open())
    }
}

#[derive(Default)]
struct DirTree {
    dirs: BTreeMap<String, DirTree>,
    files: Vec<(String, usize)>,
}

fn review_count(r: &[Rec]) -> usize {
    r.iter().filter(|x| x.review).count()
}

fn build(scan: &Scan, only_found: bool, q: &str) -> (Vec<Node>, Vec<usize>) {
    let mut root = DirTree::default();
    for (i, e) in scan.entries.iter().enumerate() {
        if only_found && e.records.is_empty() {
            continue;
        }
        let rel = scan.rel(&e.path);
        if !q.is_empty() && !rel.to_lowercase().contains(q) {
            continue;
        }
        let parts: Vec<&str> = rel.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
        let Some((file, dirs)) = parts.split_last() else { continue };
        let mut node = &mut root;
        for d in dirs {
            node = node.dirs.entry(d.to_string()).or_default();
        }
        node.files.push((file.to_string(), i));
    }
    let names: HashMap<String, usize> = scan.names.iter().enumerate().map(|(i, n)| (scan.rel(&n.path), i)).collect();
    let mut nodes = Vec::new();
    let (roots, _) = add_dir(&root, "", scan, &names, &mut nodes);
    (nodes, roots)
}

fn add_dir(t: &DirTree, prefix: &str, scan: &Scan, names: &HashMap<String, usize>, out: &mut Vec<Node>) -> (Vec<usize>, Agg) {
    let mut ids = Vec::new();
    let mut total = Agg::default();
    for (name, sub) in &t.dirs {
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}{}{name}", std::path::MAIN_SEPARATOR) };
        let at = out.len();
        let name_hit = names.get(&rel).copied();
        out.push(Node { name: name.clone(), kind: NodeKind::Dir(rel.clone()), agg: Agg::default(), name_hit, children: vec![], key: format!("D{rel}") });
        let (children, mut agg) = add_dir(sub, &rel, scan, names, out);
        if let Some(n) = name_hit {
            agg.recs += scan.names[n].records.len();
            agg.review += review_count(&scan.names[n].records);
        }
        out[at].agg = agg;
        out[at].children = children;
        total.add(&agg);
        ids.push(at);
    }
    for (name, i) in &t.files {
        let e = &scan.entries[*i];
        let name_hit = names.get(&scan.rel(&e.path)).copied();
        let mut agg = Agg { files: 1, recs: e.records.len(), review: review_count(&e.records), masked: (e.status == Status::Masked) as usize, size: e.size };
        if let Some(n) = name_hit {
            agg.recs += scan.names[n].records.len();
            agg.review += review_count(&scan.names[n].records);
        }
        let at = out.len();
        out.push(Node { name: name.clone(), kind: NodeKind::File(*i), agg, name_hit, children: vec![], key: format!("F{i}") });
        out[at].children = add_types(*i, name_hit, scan, out);
        total.add(&agg);
        ids.push(at);
    }
    (ids, total)
}

/// 파일 아래: 정보 타입별 묶음(내용 + 파일명 검출) → 값
fn add_types(file: usize, name_hit: Option<usize>, scan: &Scan, out: &mut Vec<Node>) -> Vec<usize> {
    let mut srcs = vec![Src::File(file)];
    srcs.extend(name_hit.map(Src::Name));
    let mut by: BTreeMap<&str, Vec<(Src, usize)>> = BTreeMap::new();
    for s in srcs {
        for (k, r) in scan.records(s).iter().enumerate() {
            by.entry(r.base()).or_default().push((s, k));
        }
    }
    let mut ids = Vec::new();
    for (label, items) in by {
        let review = items.iter().filter(|(s, k)| scan.records(*s)[*k].review).count();
        let at = out.len();
        let agg = Agg { recs: items.len(), review, ..Default::default() };
        out.push(Node { name: format!("{label} ({})", items.len()), kind: NodeKind::Type(file), agg, name_hit: None, children: vec![], key: format!("T{file}:{label}") });
        let mut kids = Vec::new();
        for (s, k) in items {
            let r = &scan.records(s)[k];
            kids.push(out.len());
            let agg = Agg { recs: 1, review: r.review as usize, ..Default::default() };
            out.push(Node { name: format!("{} → {}", r.original, r.masked), kind: NodeKind::Item(file, s, k), agg, name_hit: None, children: vec![], key: String::new() });
        }
        out[at].children = kids;
        ids.push(at);
    }
    ids
}

// ───────────────────────────── 색 ─────────────────────────────

fn status_tint(s: Status, review: bool) -> Retained<NSColor> {
    match (s, review) {
        (_, true) => tint(NSColor::systemYellowColor(), 0.35),
        (Status::Masked, _) => tint(NSColor::systemRedColor(), 0.22),
        (Status::Found, _) => tint(NSColor::systemPurpleColor(), 0.22),
        (Status::Skipped, _) => tint(NSColor::systemGrayColor(), 0.18),
        (Status::Error, _) => tint(NSColor::systemGrayColor(), 0.45),
        (Status::Clean, _) => NSColor::clearColor(),
    }
}

fn dir_tint() -> Retained<NSColor> {
    tint(NSColor::systemBlueColor(), 0.14)
}

fn legend(c: Retained<NSColor>, text: &str) -> Retained<NSTextField> {
    let l = kit::small_label(&format!(" {text} "));
    l.setTextColor(Some(&NSColor::labelColor()));
    l.setDrawsBackground(true);
    l.setBackgroundColor(Some(&c));
    l
}

// ───────────────────────────── 화면 ─────────────────────────────

pub struct TreeUi {
    pub view: Retained<NSView>,
    pub outline: Retained<NSOutlineView>,
    only_found: Retained<NSButton>,
    search: Retained<NSSearchField>,
    info: Retained<NSTextField>,
    split: Retained<NSSplitView>,
    empty: Retained<NSTextField>,
    hint: Retained<NSTextField>,
    title: Retained<NSTextField>,
    seg: Retained<NSSegmentedControl>,
    summary: Retained<NSTextField>,
    recs_scroll: Retained<NSScrollView>,
    pub recs: Retained<NSTableView>,
    props_scroll: Retained<NSScrollView>,
    pub props: Retained<NSTableView>,
    reveal: Retained<NSView>,
    divider_placed: std::cell::Cell<bool>,
}

impl TreeUi {
    pub fn build(ctrl: &Controller) -> TreeUi {
        let t: &AnyObject = ctrl.as_ref();
        let only_found = checkbox("정보가 있는 파일만", t, sel!(treeFilter:));
        let search = kit::search("파일 이름·경로 검색", t, sel!(treeFilter:));
        let info = kit::small_label("");
        let legends: Vec<Retained<NSTextField>> = [Status::Masked, Status::Found, Status::Skipped, Status::Error]
            .into_iter()
            .map(|s| legend(status_tint(s, false), s.label()))
            .chain([legend(status_tint(Status::Masked, true), "검토필요")])
            .collect();
        let mut bar: Vec<&NSView> = Vec::new();
        let (b1, b2) = (button("모두 펼치기", t, sel!(treeExpandAll:)), button("모두 접기", t, sel!(treeCollapseAll:)));
        let sp = kit::spacer();
        bar.extend([&*b1 as &NSView, &b2, &only_found, &sp, &search]);
        let legend_row = {
            let mut v: Vec<&NSView> = legends.iter().map(|l| -> &NSView { l }).collect();
            v.push(&info);
            let sp = kit::spacer();
            v.push(&sp);
            kit::stack(&v, false, 6.0)
        };
        let toolbar = kit::hstack(&bar);

        let (tree_scroll, outline) = kit::outline(&[
            ("name", "이름", 260.0),
            ("kind", "형식 (매직바이트)", 110.0),
            ("size", "크기", 80.0),
            ("recs", "검출", 48.0),
            ("review", "검토", 48.0),
            ("status", "상태", 60.0),
            ("note", "비고", 200.0),
        ]);
        unsafe {
            outline.setDataSource(Some(ProtocolObject::from_ref(ctrl)));
            outline.setDelegate(Some(ProtocolObject::from_ref(ctrl)));
            outline.setTarget(Some(t));
            outline.setDoubleAction(Some(sel!(treeDoubleClick:)));
        }
        let top = kit::container();
        kit::column(&top, &[(&toolbar, H::Auto), (&legend_row, H::Auto), (&tree_scroll, H::Fill)], 8.0, 6.0);

        // 상세
        let title = kit::bold_label("");
        kit::stretch(&title);
        title.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
        let labels = NSArray::from_retained_slice(&[kit::ns("검출 정보"), kit::ns("파일 정보")]);
        let seg = unsafe {
            NSSegmentedControl::segmentedControlWithLabels_trackingMode_target_action(&labels, NSSegmentSwitchTracking::SelectOne, Some(t), Some(sel!(detailTab:)), kit::mtm())
        };
        seg.setSelectedSegment(0);
        let head = kit::hstack(&[&title, &seg]);
        let summary = kit::small_label("");
        let (recs_scroll, recs) = kit::table(&[("orig", "검출된정보", 200.0), ("masked", "치환정보", 200.0), ("label", "정보타입", 150.0), ("path", "파일경로", 200.0)]);
        let (props_scroll, props) = kit::table(&[("k", "항목", 170.0), ("v", "값", 420.0)]);
        props.setHeaderView(None);
        for tv in [&recs, &props] {
            unsafe {
                tv.setDataSource(Some(ProtocolObject::from_ref(ctrl)));
                tv.setDelegate(Some(ProtocolObject::from_ref(ctrl)));
            }
        }
        let menu = NSMenu::new(kit::mtm());
        let copy = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(kit::mtm()), &kit::ns("검출된 정보 복사"), Some(sel!(detailCopy:)), &kit::ns("")) };
        unsafe { copy.setTarget(Some(t)) };
        menu.addItem(&copy);
        unsafe { recs.setMenu(Some(&menu)) };
        let reveal = kit::hstack(&[&button("원본 위치 열기", t, sel!(revealSrc:)), &button("결과 위치 열기", t, sel!(revealOut:)), &kit::spacer()]).into_super();
        let body = kit::container();
        kit::fill(&body, &recs_scroll);
        kit::fill(&body, &props_scroll);
        let hint = note("파일이나 폴더를 고르면 찾은 정보와 파일 정보가 여기에 표시됩니다. (두 번 클릭: 결과 파일 위치 열기)");
        let bottom = kit::container();
        kit::column(&bottom, &[(&head, H::Auto), (&summary, H::Auto), (&body, H::Fill), (&reveal, H::Auto)], 8.0, 6.0);
        hint.setTranslatesAutoresizingMaskIntoConstraints(false);
        bottom.addSubview(&hint);
        hint.leadingAnchor().constraintEqualToAnchor_constant(&bottom.leadingAnchor(), 12.0).setActive(true);
        hint.trailingAnchor().constraintEqualToAnchor_constant(&bottom.trailingAnchor(), -12.0).setActive(true);
        hint.topAnchor().constraintEqualToAnchor_constant(&bottom.topAnchor(), 12.0).setActive(true);

        let split = NSSplitView::new(kit::mtm());
        split.setVertical(false);
        split.setDividerStyle(objc2_app_kit::NSSplitViewDividerStyle::Thin);
        split.addArrangedSubview(&top);
        split.addArrangedSubview(&bottom);
        top.heightAnchor().constraintGreaterThanOrEqualToConstant(140.0).setActive(true);
        bottom.heightAnchor().constraintGreaterThanOrEqualToConstant(110.0).setActive(true);
        split.setHoldingPriority_forSubviewAtIndex(250.0, 0);
        split.setHoldingPriority_forSubviewAtIndex(260.0, 1);

        let empty = label("검사를 시작하면 검사한 모든 파일이 폴더 구조 그대로 여기에 표시됩니다.");
        empty.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let view = kit::container();
        kit::fill(&view, &split);
        empty.setTranslatesAutoresizingMaskIntoConstraints(false);
        view.addSubview(&empty);
        empty.centerXAnchor().constraintEqualToAnchor(&view.centerXAnchor()).setActive(true);
        empty.centerYAnchor().constraintEqualToAnchor(&view.centerYAnchor()).setActive(true);
        let ui = TreeUi { view, outline, only_found, search, info, split, empty, hint, title, seg, summary, recs_scroll, recs, props_scroll, props, reveal, divider_placed: Default::default() };
        ui.show_detail(None, 0, false);
        ui
    }

    /// 상세 창 머리 (제목, 검출 건수, 탭)
    fn show_detail(&self, title: Option<&str>, n: usize, is_file: bool) {
        let has = title.is_some();
        self.hint.setHidden(has);
        for v in [&*self.title as &NSView, &self.seg, &self.summary, &self.recs_scroll, &self.props_scroll, &self.reveal] {
            v.setHidden(!has);
        }
        if !has {
            return;
        }
        self.title.setStringValue(&kit::ns(title.unwrap_or_default()));
        self.seg.setLabel_forSegment(&kit::ns(&format!("검출 정보 ({n})")), 0);
        let records = self.seg.selectedSegment() == 0;
        self.recs_scroll.setHidden(!records);
        self.summary.setHidden(!records);
        self.props_scroll.setHidden(records);
        self.reveal.setHidden(records || !is_file);
    }

    /// 탭이 처음 보일 때 나눔선 위치 (상세 창 230pt)
    pub fn place_divider(&self) {
        if self.divider_placed.get() {
            return;
        }
        self.view.layoutSubtreeIfNeeded();
        let h = self.split.frame().size.height;
        if h > 0.0 {
            self.split.setPosition_ofDividerAtIndex((h - 230.0).max(140.0), 0);
            self.divider_placed.set(true);
        }
    }
}

// ───────────────────────────── 갱신 ─────────────────────────────

fn item_index(item: Option<&AnyObject>) -> Option<usize> {
    item?.downcast_ref::<NSNumber>().map(|n| n.as_usize())
}

pub fn reset() {
    st_mut(|s| s.tree = TreeState::default());
}

/// 검사 결과·필터가 바뀌면 트리를 다시 만들고 펼침·선택을 되살린다
pub fn refresh() {
    let u = &ui().tree;
    let only = is_on(&u.only_found);
    let q = kit::text(&u.search).trim().to_lowercase();
    let (changed, info) = st_mut(|s| {
        let Some(scan) = s.scan.as_ref() else { return (false, None) };
        let total: u64 = scan.entries.iter().map(|e| e.size).sum();
        let info = format!("파일 {}개 · {}", scan.entries.len(), kit::human(total));
        let key = (scan.version, only, q.clone());
        if s.tree.built.as_ref() == Some(&key) {
            return (false, Some(info));
        }
        let (nodes, roots) = build(scan, only, &q);
        s.tree.items = (0..nodes.len()).map(NSNumber::new_usize).collect();
        s.tree.nodes = nodes;
        s.tree.roots = roots;
        s.tree.built = Some(key);
        s.tree.quiet = true;
        (true, Some(info))
    });
    u.empty.setHidden(info.is_some());
    u.split.setHidden(info.is_none());
    u.info.setStringValue(&kit::ns(&info.unwrap_or_default()));
    if !changed {
        return;
    }
    u.outline.reloadData();
    apply_open();
    // 선택 되살리기
    let item = st(|s| {
        let i = s.tree.nodes.iter().position(|n| match (&n.kind, &s.tree.sel) {
            (NodeKind::File(a), Some(Sel::File(b))) => a == b,
            (NodeKind::Dir(a), Some(Sel::Dir(b))) => a == b,
            _ => false,
        })?;
        Some(s.tree.items[i].clone())
    });
    let row = item.map(|it| unsafe { u.outline.rowForItem(Some(&it)) });
    match row.filter(|r| *r >= 0) {
        Some(r) => u.outline.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(r as usize), false),
        None => unsafe { u.outline.deselectAll(None) },
    }
    st_mut(|s| s.tree.quiet = false);
    selection_changed();
}

/// 기억한 펼침 상태를 위에서부터 적용 (알림은 quiet 로 무시)
fn apply_open() {
    let plan: Vec<(Retained<NSNumber>, bool)> = st(|s| {
        let mut plan = Vec::new();
        let mut stack: Vec<usize> = s.tree.roots.iter().rev().copied().collect();
        while let Some(i) = stack.pop() {
            if s.tree.nodes[i].children.is_empty() {
                continue;
            }
            let open = s.tree.is_open(i);
            plan.push((s.tree.items[i].clone(), open));
            if open {
                stack.extend(s.tree.nodes[i].children.iter().rev());
            }
        }
        plan
    });
    let ov = &ui().tree.outline;
    for (item, open) in plan {
        unsafe {
            if open {
                ov.expandItem_expandChildren(Some(&item), false);
            } else {
                ov.collapseItem_collapseChildren(Some(&item), false);
            }
        }
    }
}

pub fn expand_all(open: bool) {
    st_mut(|s| {
        let t = &mut s.tree;
        for n in &t.nodes {
            if !n.children.is_empty() {
                t.open.insert(n.key.clone(), open);
            }
        }
        t.quiet = true;
    });
    let ov = &ui().tree.outline;
    unsafe {
        if open {
            ov.expandItem_expandChildren(None, true);
        } else {
            ov.collapseItem_collapseChildren(None, true);
        }
    }
    st_mut(|s| s.tree.quiet = false);
}

// ───────────────────────────── NSOutlineView 데이터 ─────────────────────────────

pub fn children_count(item: Option<&AnyObject>) -> isize {
    st(|s| match item_index(item) {
        None => s.tree.roots.len(),
        Some(i) => s.tree.nodes.get(i).map_or(0, |n| n.children.len()),
    }) as isize
}

pub fn child(index: isize, item: Option<&AnyObject>) -> Retained<AnyObject> {
    let n = st(|s| {
        let list = match item_index(item) {
            None => &s.tree.roots,
            Some(i) => &s.tree.nodes[i].children,
        };
        s.tree.items[list[index as usize]].clone()
    });
    Retained::into_super(Retained::into_super(Retained::into_super(n))).into()
}

pub fn expandable(item: &AnyObject) -> bool {
    st(|s| item_index(Some(item)).and_then(|i| s.tree.nodes.get(i)).is_some_and(|n| !n.children.is_empty()))
}

pub fn view_for(ov: &NSOutlineView, col: Option<&NSTableColumn>, item: &AnyObject) -> Option<Retained<NSView>> {
    let (text, look, badge) = st(|s| {
        let scan = s.scan.as_ref()?;
        let n = s.tree.nodes.get(item_index(Some(item))?)?;
        let own = matches!(n.kind, NodeKind::Dir(_) | NodeKind::File(_));
        let is_item = matches!(n.kind, NodeKind::Item(..));
        let item_review = match n.kind {
            NodeKind::Item(_, src, k) => scan.records(src)[k].review,
            _ => false,
        };
        let num = |v: usize, warn: bool| (if v > 0 && !is_item { v.to_string() } else { String::new() }, Look { bold: true, warn, right: true, ..PLAIN });
        Some(match col_id(col).as_str() {
            "name" => {
                let look = match n.kind {
                    NodeKind::Dir(_) | NodeKind::Type(_) => Look { bold: true, ..PLAIN },
                    NodeKind::Item(..) => Look { mono: true, warn: item_review, ..PLAIN },
                    NodeKind::File(_) => PLAIN,
                };
                (n.name.clone(), look, n.name_hit.map(|_| "이름"))
            }
            "kind" => (
                match &n.kind {
                    NodeKind::Dir(_) => "폴더".into(),
                    NodeKind::File(i) => scan.entries[*i].kind.clone(),
                    _ => String::new(),
                },
                Look { dim: true, ..PLAIN },
                None,
            ),
            "size" => (if own { kit::human(n.agg.size) } else { String::new() }, Look { right: true, ..PLAIN }, None),
            "recs" => {
                let (t, l) = num(n.agg.recs, false);
                (t, l, None)
            }
            "review" => {
                let (t, l) = num(n.agg.review, true);
                (t, l, None)
            }
            "status" => (
                match &n.kind {
                    NodeKind::File(i) => scan.entries[*i].status.label().to_string(),
                    NodeKind::Item(..) if item_review => "검토필요".into(),
                    _ => String::new(),
                },
                PLAIN,
                None,
            ),
            _ => (
                match &n.kind {
                    NodeKind::Dir(_) => format!("파일 {}개", n.agg.files),
                    NodeKind::File(i) => scan.entries[*i].notes.join(" / "),
                    NodeKind::Item(_, src, k) => scan.records(*src)[*k].label.clone(),
                    NodeKind::Type(_) => String::new(),
                },
                Look { dim: true, ..PLAIN },
                None,
            ),
        })
    })?;
    Some(cell(ov, col, &text, look, badge))
}

/// 행 배경 = 파일 상태 색
pub fn row_tint(row_view: &NSTableRowView, row: isize) {
    let ov = &ui().tree.outline;
    let item = ov.itemAtRow(row);
    let color = st(|s| {
        let scan = s.scan.as_ref()?;
        let n = s.tree.nodes.get(item_index(item.as_deref())?)?;
        match &n.kind {
            NodeKind::Dir(_) if n.agg.recs > 0 => Some(dir_tint()),
            NodeKind::File(i) => {
                let e = &scan.entries[*i];
                (e.status != Status::Clean).then(|| status_tint(e.status, review_count(&e.records) > 0))
            }
            _ => None,
        }
    });
    row_view.setBackgroundColor(&color.unwrap_or_else(NSColor::clearColor));
}

pub fn did_expand(item: Option<&AnyObject>, open: bool) {
    let Some(i) = item_index(item) else { return };
    // 사용자가 펼친 경우: 기억하고, 기억해 둔 하위 펼침을 되살린다
    let kids: Option<Vec<Retained<NSNumber>>> = st_mut(|s| {
        if s.tree.quiet {
            return None;
        }
        let key = s.tree.nodes.get(i)?.key.clone();
        s.tree.open.insert(key, open);
        open.then(|| s.tree.nodes[i].children.iter().filter(|c| !s.tree.nodes[**c].children.is_empty() && s.tree.is_open(**c)).map(|c| s.tree.items[*c].clone()).collect())
    });
    for k in kids.unwrap_or_default() {
        unsafe { ui().tree.outline.expandItem_expandChildren(Some(&k), false) };
    }
}

/// 선택한 행 → 상세 창
pub fn selection_changed() {
    let u = &ui().tree;
    let row = u.outline.selectedRow();
    let item = (row >= 0).then(|| u.outline.itemAtRow(row)).flatten();
    let head = st_mut(|s| {
        if s.tree.quiet {
            return None;
        }
        let scan = s.scan.as_ref()?;
        let sel = item_index(item.as_deref()).and_then(|i| s.tree.nodes.get(i)).map(|n| match &n.kind {
            NodeKind::Dir(p) => Sel::Dir(p.clone()),
            NodeKind::File(i) | NodeKind::Type(i) | NodeKind::Item(i, ..) => Sel::File(*i),
        });
        let (title, recs, props, is_file) = match &sel {
            None => return Some((None, 0, false)),
            Some(Sel::File(i)) => {
                let e = &scan.entries[*i];
                let mut v: Vec<(Src, usize)> = (0..e.records.len()).map(|k| (Src::File(*i), k)).collect();
                if let Some(n) = scan.names.iter().position(|n| n.path == e.path) {
                    v.extend((0..scan.names[n].records.len()).map(|k| (Src::Name(n), k)));
                }
                (scan.rel(&e.path), v, file_props(scan, *i), true)
            }
            Some(Sel::Dir(p)) => {
                let prefix = format!("{p}{}", std::path::MAIN_SEPARATOR);
                let under = |path: &std::path::Path| {
                    let r = scan.rel(path);
                    r == *p || r.starts_with(&prefix)
                };
                let mut v = Vec::new();
                for (i, e) in scan.entries.iter().enumerate() {
                    if under(&e.path) {
                        v.extend((0..e.records.len()).map(|k| (Src::File(i), k)));
                    }
                }
                for (i, n) in scan.names.iter().enumerate() {
                    if under(&n.path) {
                        v.extend((0..n.records.len()).map(|k| (Src::Name(i), k)));
                    }
                }
                let agg = s.tree.nodes.iter().find(|n| matches!(&n.kind, NodeKind::Dir(x) if x == p)).map(|n| n.agg).unwrap_or_default();
                let props = vec![
                    ("원본 경로".into(), scan.root.join(p).display().to_string()),
                    ("파일 수".into(), agg.files.to_string()),
                    ("전체 크기".into(), kit::human(agg.size)),
                    ("치환한 파일".into(), agg.masked.to_string()),
                    ("검출".into(), format!("{}건 (검토필요 {}건)", agg.recs, agg.review)),
                ];
                (prefix, v, props, false)
            }
        };
        let n = recs.len();
        s.tree.detail_path = !is_file;
        s.tree.detail = recs;
        s.tree.props = props;
        s.tree.sel = sel;
        Some((Some(title), n, is_file))
    });
    let Some((title, n, is_file)) = head else { return };
    // 타입별 건수 요약
    let summary = st(|s| {
        let scan = s.scan.as_ref()?;
        let mut by: BTreeMap<&str, usize> = BTreeMap::new();
        for (src, k) in &s.tree.detail {
            *by.entry(scan.records(*src)[*k].base()).or_default() += 1;
        }
        Some(by.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join("  ·  "))
    });
    u.summary.setStringValue(&kit::ns(&summary.unwrap_or_default()));
    u.recs.tableColumns().objectAtIndex(3).setHidden(!st(|s| s.tree.detail_path));
    u.show_detail(title.as_deref(), n, is_file);
    u.recs.reloadData();
    u.props.reloadData();
}

fn file_props(scan: &Scan, i: usize) -> Vec<(String, String)> {
    let e = &scan.entries[i];
    let ext = crate::exts::ext_of(&e.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let catalog = ext.as_deref().and_then(crate::exts::lookup).map(|x| format!("{} — {}", x.category, x.desc)).unwrap_or_else(|| "목록에 없음".into());
    let mut rows: Vec<(String, String)> = vec![
        ("원본 경로".into(), e.path.display().to_string()),
        ("결과 경로".into(), e.out.display().to_string()),
        ("실제 형식 (매직바이트)".into(), e.kind.clone()),
        ("확장자".into(), ext.map(|x| format!(".{x}")).unwrap_or_else(|| "없음".into())),
        ("확장자 분류".into(), catalog),
        ("크기".into(), format!("{} ({} bytes)", kit::human(e.size), e.size)),
        ("상태".into(), e.status.label().to_string()),
        ("검출".into(), format!("{}건 (검토필요 {}건)", e.records.len(), review_count(&e.records))),
    ];
    rows.extend(e.notes.iter().map(|n| ("알림".to_string(), n.clone())));
    rows
}

pub fn detail_tab() {
    let u = &ui().tree;
    let (title, n, is_file) = st(|s| (s.tree.sel.is_some(), s.tree.detail.len(), matches!(s.tree.sel, Some(Sel::File(_)))));
    let t = kit::text(&u.title);
    u.show_detail(title.then_some(t.as_str()), n, is_file);
}

pub fn recs_count() -> isize {
    st(|s| s.tree.detail.len() as isize)
}

pub fn props_count() -> isize {
    st(|s| s.tree.props.len() as isize)
}

pub fn recs_view(tv: &NSTableView, col: Option<&NSTableColumn>, row: usize) -> Option<Retained<NSView>> {
    let (text, look) = st(|s| {
        let scan = s.scan.as_ref()?;
        let (src, k) = *s.tree.detail.get(row)?;
        let r = &scan.records(src)[k];
        Some(match col_id(col).as_str() {
            "orig" => (r.original.clone(), Look { mono: true, warn: r.review, ..PLAIN }),
            "masked" => (r.masked.clone(), Look { mono: true, ..PLAIN }),
            "label" => (r.label.clone(), Look { warn: r.review, ..PLAIN }),
            _ => (scan.rel(scan.path(src)), PLAIN),
        })
    })?;
    Some(cell(tv, col, &text, look, None))
}

pub fn props_view(tv: &NSTableView, col: Option<&NSTableColumn>, row: usize) -> Option<Retained<NSView>> {
    let (k, v) = st(|s| s.tree.props.get(row).cloned())?;
    Some(if col_id(col) == "k" { cell(tv, col, &k, Look { dim: true, ..PLAIN }, None) } else { cell(tv, col, &v, PLAIN, None) })
}

pub fn detail_copy() {
    let Some(r) = kit::row(ui().tree.recs.clickedRow()) else { return };
    if let Some(t) = st(|s| s.tree.detail.get(r).and_then(|(src, k)| Some(s.scan.as_ref()?.records(*src)[*k].original.clone()))) {
        kit::copy_text(&t);
    }
}

/// 선택한 파일의 원본(false)·결과(true) 위치 열기
pub fn reveal(out: bool) {
    let p = st(|s| match (&s.tree.sel, s.scan.as_ref()) {
        (Some(Sel::File(i)), Some(scan)) => Some(if out { scan.entries[*i].out.clone() } else { scan.entries[*i].path.clone() }),
        _ => None,
    });
    if let Some(p) = p {
        sys::reveal(&p);
    }
}

/// 두 번 클릭: 폴더·묶음은 펼치기/접기, 파일은 결과 위치 열기
pub fn double_click() {
    let ov = &ui().tree.outline;
    let row = ov.clickedRow();
    if row < 0 {
        return;
    }
    let Some(item) = ov.itemAtRow(row) else { return };
    let action = st(|s| {
        let n = s.tree.nodes.get(item_index(Some(&item))?)?;
        Some(match &n.kind {
            NodeKind::File(i) => Err(s.scan.as_ref()?.entries[*i].out.clone()),
            _ => Ok(!n.children.is_empty()),
        })
    });
    match action {
        Some(Ok(true)) => unsafe {
            if ov.isItemExpanded(Some(&item)) {
                ov.collapseItem_collapseChildren(Some(&item), false);
            } else {
                ov.expandItem_expandChildren(Some(&item), false);
            }
        },
        Some(Err(p)) => sys::reveal(&p),
        _ => {}
    }
}

//! 리스트뷰 공통 — 가상(LVS_OWNERDATA) 목록은 데이터를 갖지 않고, 화면에 보이는 칸만 LVN_GETDISPINFOW 로 묻는다.

use super::ctl::{child, send, FontReg};
use super::dpi::{px, FontKind};
use windows::core::PWSTR;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::{WS_TABSTOP, WS_EX_CLIENTEDGE};

pub struct Col {
    pub title: &'static str,
    /// 96 DPI 기준 폭 (0 = 남은 폭 전부)
    pub w: i32,
    pub right: bool,
}

pub const fn col(title: &'static str, w: i32) -> Col {
    Col { title, w, right: false }
}

pub const fn col_r(title: &'static str, w: i32) -> Col {
    Col { title, w, right: true }
}

pub fn create(parent: HWND, id: u16, owner_data: bool, extra: u32, cols: &[Col], dpi: u32, reg: &mut FontReg, font: FontKind) -> HWND {
    let mut style = LVS_REPORT | LVS_SHOWSELALWAYS | LVS_SINGLESEL | LVS_SHAREIMAGELISTS | WS_TABSTOP.0 | extra;
    if owner_data {
        style |= LVS_OWNERDATA;
    }
    let h = child(parent, WC_LISTVIEWW, "", style, WS_EX_CLIENTEDGE.0, id, reg, font);
    let ex = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_LABELTIP;
    send(h, LVM_SETEXTENDEDLISTVIEWSTYLE, ex as usize, ex as isize);
    for (i, c) in cols.iter().enumerate() {
        let mut t: Vec<u16> = c.title.encode_utf16().chain([0]).collect();
        let lc = LVCOLUMNW {
            mask: LVCF_TEXT | LVCF_WIDTH | LVCF_FMT,
            fmt: if c.right { LVCFMT_RIGHT } else { LVCFMT_LEFT },
            cx: px(if c.w > 0 { c.w } else { 200 }, dpi),
            pszText: PWSTR(t.as_mut_ptr()),
            ..Default::default()
        };
        send(h, LVM_INSERTCOLUMNW, i, &lc as *const _ as isize);
    }
    h
}

/// DPI 가 바뀌거나 창 크기가 바뀌면 열 폭 다시 계산 (w=0 인 열이 남은 폭을 채움)
pub fn fit_columns(h: HWND, cols: &[Col], total: i32, dpi: u32) {
    let fixed: i32 = cols.iter().filter(|c| c.w > 0).map(|c| px(c.w, dpi)).sum();
    let scrollbar = px(22, dpi);
    for (i, c) in cols.iter().enumerate() {
        let w = if c.w > 0 { px(c.w, dpi) } else { (total - fixed - scrollbar).max(px(80, dpi)) };
        send(h, LVM_SETCOLUMNWIDTH, i, w as isize);
    }
}

pub fn set_count(h: HWND, n: usize) {
    send(h, LVM_SETITEMCOUNT, n, (LVSICF_NOSCROLL | LVSICF_NOINVALIDATEALL) as isize);
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(h, None, false);
    }
}

/// 가상 목록에 체크 상자(상태 이미지) 사용: 상태는 LVN_GETDISPINFOW 의 LVIF_STATE 로 묻는다
pub fn use_states(h: HWND, states: HIMAGELIST) {
    send(h, LVM_SETCALLBACKMASK, LVIS_STATEIMAGEMASK.0 as usize, 0);
    send(h, LVM_SETIMAGELIST, LVSIL_STATE as usize, states.0 as isize);
}

/// 여러 행 선택 허용 (Ctrl·Shift 로 골라 한 번에 체크 해제)
pub fn multi_select(h: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_STYLE};
    unsafe {
        let s = GetWindowLongPtrW(h, GWL_STYLE);
        SetWindowLongPtrW(h, GWL_STYLE, s & !(LVS_SINGLESEL as isize));
    }
}

/// 선택된 행 번호 (가상 목록도 동작)
pub fn selected_rows(h: HWND) -> Vec<usize> {
    let mut v = Vec::new();
    let mut i = -1isize;
    loop {
        i = send(h, LVM_GETNEXTITEM, i as usize, LVNI_SELECTED as isize);
        if i < 0 {
            return v;
        }
        v.push(i as usize);
    }
}

/// 체크 상자(상태 이미지) 위를 누른 행
pub fn state_hit(h: HWND, pt: POINT) -> Option<usize> {
    let mut ht = LVHITTESTINFO { pt, ..Default::default() };
    let r = send(h, LVM_HITTEST, 0, &mut ht as *mut _ as isize);
    (r >= 0 && (ht.flags & LVHT_ONITEMSTATEICON).0 != 0).then_some(r as usize)
}

/// 상태 이미지 번호를 가상 목록 칸 요청에 채움
pub fn put_state(item: &mut LVITEMW, index: u32) {
    if (item.mask & LVIF_STATE).0 != 0 {
        item.stateMask = LVIS_STATEIMAGEMASK;
        item.state = LIST_VIEW_ITEM_STATE_FLAGS(index << 12);
    }
}

pub fn redraw(h: HWND) {
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(h, None, false);
    }
}

pub fn select(h: HWND, i: usize) {
    let it = LVITEMW { stateMask: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0), state: LIST_VIEW_ITEM_STATE_FLAGS(LVIS_SELECTED.0 | LVIS_FOCUSED.0), ..Default::default() };
    send(h, LVM_SETITEMSTATE, i, &it as *const _ as isize);
    send(h, LVM_ENSUREVISIBLE, i, 0);
}

pub fn ensure_visible(h: HWND, i: usize) {
    send(h, LVM_ENSUREVISIBLE, i, 0);
}

/// 가상 목록의 칸 글자: 앱이 가진 버퍼를 가리키게 한다 (다음 알림까지 유효 — 길이 제한 없음)
pub fn put_text(item: &mut LVITEMW, s: &str, buf: &mut Vec<u16>) {
    buf.clear();
    buf.extend(s.encode_utf16());
    buf.push(0);
    item.pszText = PWSTR(buf.as_mut_ptr());
}

/// 클릭 위치의 (행, 열, 이미지 위인가)
pub fn hit(h: HWND, pt: POINT) -> Option<(usize, usize, bool)> {
    let mut ht = LVHITTESTINFO { pt, ..Default::default() };
    let r = send(h, LVM_SUBITEMHITTEST, 0, &mut ht as *mut _ as isize);
    // 아이콘 위인지는 LVM_HITTEST 로 판정 (보고서 보기에서 LVM_SUBITEMHITTEST 는 아이콘 플래그를 주지 않음)
    let mut it = LVHITTESTINFO { pt, ..Default::default() };
    send(h, LVM_HITTEST, 0, &mut it as *mut _ as isize);
    let on_icon = (it.flags & LVHT_ONITEMICON).0 != 0 && (it.flags & LVHT_ONITEMSTATEICON).0 == 0;
    (r >= 0 && ht.iItem >= 0).then(|| (ht.iItem as usize, ht.iSubItem.max(0) as usize, on_icon))
}

// ── 일반(비가상) 목록: 정보 창, 확장자 창, 미리보기, 파일 정보 ──

pub fn clear(h: HWND) {
    send(h, LVM_DELETEALLITEMS, 0, 0);
}

pub fn push(h: HWND, cells: &[&str]) -> usize {
    let n = send(h, LVM_GETITEMCOUNT, 0, 0) as i32;
    let mut first: Vec<u16> = cells.first().copied().unwrap_or("").encode_utf16().chain([0]).collect();
    let it = LVITEMW { mask: LVIF_TEXT, iItem: n, pszText: PWSTR(first.as_mut_ptr()), ..Default::default() };
    let row = send(h, LVM_INSERTITEMW, 0, &it as *const _ as isize).max(0) as usize;
    for (k, c) in cells.iter().enumerate().skip(1) {
        let mut t: Vec<u16> = c.encode_utf16().chain([0]).collect();
        let it = LVITEMW { iSubItem: k as i32, pszText: PWSTR(t.as_mut_ptr()), ..Default::default() };
        send(h, LVM_SETITEMTEXTW, row, &it as *const _ as isize);
    }
    row
}

pub fn set_checked(h: HWND, i: usize, on: bool) {
    let it = LVITEMW { stateMask: LVIS_STATEIMAGEMASK, state: LIST_VIEW_ITEM_STATE_FLAGS(if on { 2 } else { 1 } << 12), ..Default::default() };
    send(h, LVM_SETITEMSTATE, i, &it as *const _ as isize);
}

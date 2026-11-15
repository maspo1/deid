//! GDI·USER 핸들 RAII — 만든 핸들은 모두 여기 타입에 담아 Drop 에서 해제한다 (누수 원천 차단).
//!
//! 시스템 브러시(`GetSysColorBrush`)·공유 아이콘(`LoadIconW`)은 OS 소유라 해제하면 안 되므로 감싸지 않는다.
//! 그래서 이 앱이 직접 만드는 GDI 핸들은 글꼴, DIB 비트맵, 이미지 목록, 팝업 메뉴뿐이다.

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateDIBSection, CreateFontIndirectW, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY,
    DEFAULT_CHARSET, DIB_RGB_COLORS, FW_BOLD, FW_NORMAL, HBITMAP, HFONT, LOGFONTW, OUT_TT_PRECIS,
};
use windows::Win32::UI::Controls::{ImageList_Add, ImageList_Create, ImageList_Destroy, HIMAGELIST, ILC_COLOR32};
use windows::Win32::UI::WindowsAndMessaging::{AppendMenuW, CreatePopupMenu, DestroyMenu, HMENU, MF_SEPARATOR, MF_STRING};

/// 글꼴
pub struct Font(HFONT);

impl Font {
    /// `pt` 포인트 크기를 `dpi` 에 맞춘 화소 높이로 (ClearType)
    pub fn new(face: &str, pt: i32, bold: bool, dpi: u32) -> Font {
        let mut lf = LOGFONTW {
            lfHeight: -((pt as i64 * dpi as i64 + 36) / 72) as i32,
            lfWeight: if bold { FW_BOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
            lfCharSet: DEFAULT_CHARSET,
            lfOutPrecision: OUT_TT_PRECIS,
            lfQuality: CLEARTYPE_QUALITY,
            ..Default::default()
        };
        for (d, s) in lf.lfFaceName.iter_mut().zip(face.encode_utf16().take(31)) {
            *d = s;
        }
        Font(unsafe { CreateFontIndirectW(&lf) })
    }

    pub fn handle(&self) -> HFONT {
        self.0
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.0);
            }
        }
    }
}

/// 32비트 DIB 비트맵 (이미지 목록 재료, 정보 창 로고)
pub struct Bitmap(HBITMAP);

impl Bitmap {
    /// 위에서 아래 순서의 BGRA 화소로 만듦
    pub fn from_bgra(w: i32, h: i32, px: &[u32]) -> Option<Bitmap> {
        debug_assert_eq!(px.len(), (w * h) as usize);
        let bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 위에서 아래로
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        unsafe {
            let bmp = Bitmap(CreateDIBSection(None, &bi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?);
            if bits.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
            Some(bmp)
        }
    }

    pub fn handle(&self) -> HBITMAP {
        self.0
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(self.0);
            }
        }
    }
}

/// 이미지 목록 (리스트뷰에는 LVS_SHAREIMAGELISTS 로 넘기므로 소유권은 여기)
pub struct ImageList(HIMAGELIST);

impl ImageList {
    pub fn handle(&self) -> HIMAGELIST {
        self.0
    }
}

impl Drop for ImageList {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = ImageList_Destroy(self.0);
            }
        }
    }
}

/// 체크 상자 이미지 3개 (리스트뷰 상태 이미지 1·2·3 = `exclude::Check`): 0 = 해제, 1 = 체크, 2 = 섞임
pub fn check_boxes(size: i32) -> ImageList {
    let size = size.max(9);
    let (w, h) = (size * 3, size);
    let mut px = vec![0u32; (w * h) as usize];
    let b = size * 13 / 16;
    let o = (size - b) / 2;
    let blend = |a: f64| {
        // 흰 바탕 위 진한 잉크(#202020)를 덮은 정도 a 만큼 섞은 불투명 색
        let c = (255.0 - (255.0 - 32.0) * a.clamp(0.0, 1.0)).round() as u32;
        0xFF00_0000 | c << 16 | c << 8 | c
    };
    let seg = |px_: f64, py: f64, (ax, ay): (f64, f64), (bx, by): (f64, f64)| {
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((px_ - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((px_ - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
    };
    let bf = b as f64;
    let (p1, p2, p3) = ((0.20 * bf, 0.52 * bf), (0.42 * bf, 0.74 * bf), (0.80 * bf, 0.28 * bf));
    let half = (bf * 0.075).max(0.8);
    for img in 0..3 {
        for y in 0..b {
            for x in 0..b {
                let edge = x == 0 || y == 0 || x == b - 1 || y == b - 1;
                let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
                let c = if edge {
                    0xFF33_3333
                } else if img == 1 {
                    let d = seg(fx, fy, p1, p2).min(seg(fx, fy, p2, p3));
                    blend(half + 0.5 - d)
                } else if img == 2 && x >= b / 4 && y >= b / 4 && x < b - b / 4 && y < b - b / 4 {
                    0xFF33_3333
                } else {
                    0xFFFF_FFFF
                };
                px[((o + y) * w + img * size + o + x) as usize] = c;
            }
        }
    }
    unsafe {
        let list = ImageList(ImageList_Create(size, size, ILC_COLOR32, 3, 0));
        if let Some(bmp) = Bitmap::from_bgra(w, h, &px) {
            ImageList_Add(list.0, bmp.0, HBITMAP::default());
        }
        list
    }
}

/// 트리 펼치기 상자 이미지 3개: 0 = 빈칸, 1 = [+], 2 = [−]. `size` 는 DPI 를 반영한 화소 크기.
pub fn tree_boxes(size: i32) -> ImageList {
    let size = size.max(9);
    let (w, h) = (size * 3, size);
    let mut px = vec![0u32; (w * h) as usize]; // BGRA (사전 곱셈), 0 = 투명
    // 공용 컨트롤 v6 는 알파가 전부 0 인 이미지를 "알파 없음"으로 보고 검게 칠한다 →
    // 빈칸 이미지에 알파 1/255 점 하나 (눈에 안 보임)
    px[0] = 0x0100_0000;
    let b = (size * 9 / 16) | 1; // 상자 한 변 (홀수 → 가운데 선이 칼같이)
    let o = (size - b) / 2;
    let border = 0xFF80_8080u32;
    let fill = 0xFFFF_FFFFu32;
    let ink = 0xFF20_2020u32;
    for img in 1..3 {
        let x0 = img * size + o;
        for y in 0..b {
            for x in 0..b {
                let edge = x == 0 || y == 0 || x == b - 1 || y == b - 1;
                let mid = b / 2;
                let horiz = y == mid && x >= 2 && x <= b - 3;
                let vert = img == 1 && x == mid && y >= 2 && y <= b - 3;
                px[((o + y) * w + x0 + x) as usize] = if edge {
                    border
                } else if horiz || vert {
                    ink
                } else {
                    fill
                };
            }
        }
    }
    unsafe {
        let list = ImageList(ImageList_Create(size, size, ILC_COLOR32, 3, 0));
        if let Some(bmp) = Bitmap::from_bgra(w, h, &px) {
            ImageList_Add(list.0, bmp.0, HBITMAP::default());
        } // bmp 는 여기서 해제 (이미지 목록이 복사본을 가짐)
        list
    }
}

/// 팝업 메뉴 (우클릭)
pub struct PopupMenu(HMENU);

impl PopupMenu {
    pub fn new(items: &[(u16, &str)]) -> PopupMenu {
        let m = PopupMenu(unsafe { CreatePopupMenu() }.unwrap_or_default());
        for (id, text) in items {
            let w: Vec<u16> = text.encode_utf16().chain([0]).collect();
            unsafe {
                if *id == 0 {
                    let _ = AppendMenuW(m.0, MF_SEPARATOR, 0, None); // id 0 = 구분선
                } else {
                    let _ = AppendMenuW(m.0, MF_STRING, *id as usize, PCWSTR(w.as_ptr()));
                }
            }
        }
        m
    }

    pub fn handle(&self) -> HMENU {
        self.0
    }
}

impl Drop for PopupMenu {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DestroyMenu(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetGuiResources, GR_GDIOBJECTS, GR_USEROBJECTS};

    fn counts() -> (u32, u32) {
        unsafe { (GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS), GetGuiResources(GetCurrentProcess(), GR_USEROBJECTS)) }
    }

    /// 만들고 버리기를 반복해도 프로세스의 GDI·USER 핸들 수가 늘지 않아야 한다
    #[test]
    fn no_handle_leaks() {
        // 첫 생성 시 OS 내부 캐시가 생길 수 있으므로 한 번 데운 뒤 측정
        drop((Font::new("Malgun Gothic", 9, false, 96), tree_boxes(16), PopupMenu::new(&[(1, "a")])));
        let before = counts();
        for i in 0..300 {
            let dpi = 96 + (i % 5) * 24;
            let f = Font::new("Malgun Gothic", 9, i % 2 == 0, dpi);
            let m = Font::new("Consolas", 9, false, dpi);
            assert!(!f.handle().is_invalid() && !m.handle().is_invalid());
            let l = tree_boxes((16 * dpi / 96) as i32);
            assert!(!l.handle().is_invalid());
            let p = PopupMenu::new(&[(1, "원본 위치"), (2, "복사")]);
            assert!(!p.handle().is_invalid());
        }
        assert_eq!(counts(), before, "GDI/USER 핸들 누수");
    }

    #[test]
    fn font_height_scales_with_dpi() {
        let h = |dpi| {
            let f = Font::new("Malgun Gothic", 9, false, dpi);
            let mut lf = LOGFONTW::default();
            unsafe {
                windows::Win32::Graphics::Gdi::GetObjectW(
                    f.handle(),
                    std::mem::size_of::<LOGFONTW>() as i32,
                    Some(&mut lf as *mut _ as *mut core::ffi::c_void),
                );
            }
            lf.lfHeight
        };
        assert_eq!(h(96), -12);
        assert_eq!(h(144), -18);
        assert_eq!(h(192), -24);
    }
}

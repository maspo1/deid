//! 왼쪽 설정 패널 — 검출 항목, 탐지 엔진, 그림(OCR), 파일, 사용자 정의 정규식.
//! 컨트롤이 곧 설정 값이다. 검사 시작·미리보기 때 `read` 로 `Settings` 를 만든다.

use super::kit::{self, bold_label, button, checkbox, field, is_on, label, note, set_on, small_label, stretch, text};
use super::Controller;
use crate::common::InfoType;
use crate::exts::ExtFilter;
use crate::gui::settings::Settings;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{sel, Message};
use objc2_app_kit::{NSButton, NSColor, NSControl, NSScrollView, NSSlider, NSTextField, NSTextView, NSView};

pub struct OptionsUi {
    pub scroll: Retained<NSScrollView>,
    types: Vec<Retained<NSButton>>,
    ml: Retained<NSButton>,
    threshold: Retained<NSSlider>,
    threshold_lbl: Retained<NSTextField>,
    ocr: Retained<NSButton>,
    ocr_scales: Retained<NSTextField>,
    ocr_mp: Retained<NSTextField>,
    blur: Retained<NSSlider>,
    blur_lbl: Retained<NSTextField>,
    rename: Retained<NSButton>,
    scan_exe: Retained<NSButton>,
    max_mb: Retained<NSTextField>,
    min_run: Retained<NSTextField>,
    pub ext_btn: Retained<NSButton>,
    pub patterns: Retained<NSTextView>,
    pattern_err: Retained<NSTextField>,
    /// 검사 중 잠글 컨트롤
    controls: Vec<Retained<NSControl>>,
    /// OCR 을 끄면 잠글 컨트롤
    ocr_controls: Vec<Retained<NSControl>>,
}

fn slider(min: f64, max: f64, t: &AnyObject) -> Retained<NSSlider> {
    let s = unsafe { NSSlider::sliderWithValue_minValue_maxValue_target_action(min, min, max, Some(t), Some(sel!(optionChanged:)), kit::mtm()) };
    s.setContinuous(true);
    kit::width(&s, 150.0);
    s
}

fn num_field(t: &AnyObject) -> Retained<NSTextField> {
    let f = field("", "");
    kit::width(&f, 70.0);
    unsafe {
        f.setTarget(Some(t));
        f.setAction(Some(sel!(optionChanged:)));
    }
    f
}

/// "라벨  컨트롤 …" 한 줄 (라벨 폭 고정으로 줄 맞춤)
fn row(name: &str, views: &[&NSView]) -> Retained<NSView> {
    let l = label(name);
    kit::width(&l, 150.0);
    let mut all: Vec<&NSView> = vec![&l];
    all.extend_from_slice(views);
    kit::hstack(&all).into_super()
}

fn up(c: &Retained<impl AsRef<NSControl>>) -> Retained<NSControl> {
    let r: &NSControl = (**c).as_ref();
    r.retain()
}

impl OptionsUi {
    pub fn build(ctrl: &Controller) -> OptionsUi {
        let t: &AnyObject = ctrl.as_ref();
        let changed = sel!(optionChanged:);

        // 검출 항목 (개인정보 6 + 민감정보)
        let types: Vec<Retained<NSButton>> = InfoType::BUILTIN.iter().map(|ty| checkbox(ty.label(), t, changed)).collect();
        let (pi, si) = types.split_at(6);
        let col = |title: &str, items: &[Retained<NSButton>]| {
            let head = small_label(title);
            let mut v: Vec<&NSView> = vec![&head];
            v.extend(items.iter().map(|b| -> &NSView { b }));
            kit::stack(&v, true, 4.0)
        };
        let (c1, c2) = (col("개인정보", pi), col("민감정보", si));
        c1.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
        let types_row = kit::stack(&[&c1, &c2], false, 16.0);
        types_row.setAlignment(objc2_app_kit::NSLayoutAttribute::Top);
        let all_btns = kit::hstack(&[&button("모두 선택", t, sel!(selectAllTypes:)), &button("모두 해제", t, sel!(clearAllTypes:))]);

        // 탐지 엔진
        let ml = checkbox("XGBoost 맥락 탐지", t, changed);
        ml.setToolTip(Some(&kit::ns(
            "기호가 바뀌거나 빠진 번호(192-168-0-1, 0109999-1234 등)를 맥락으로 추론합니다.\n꺼도 정규식·사전 규칙과 010+8자리 기계적 치환은 동작합니다.",
        )));
        let threshold = slider(0.5, 0.99, t);
        threshold.setToolTip(Some(&kit::ns("이 확률 이상이면 확정 치환, 0.5 ~ 이 값 사이는 치환 후 '검토필요'로 표시 (기본 0.70)")));
        let threshold_lbl = label("");
        let thr_row = row("채택 확률", &[&threshold, &threshold_lbl]);

        // 그림 (OCR)
        let ocr = checkbox("그림 속 글자 검사 (macOS 내장 OCR) — 찾은 정보는 흐리게", t, changed);
        let ocr_scales = num_field(t);
        ocr_scales.setToolTip(Some(&kit::ns("작은 글자 인식용 확대 배율. 쉼표로 여러 개 (앞쪽 우선)")));
        let ocr_mp = num_field(t);
        let blur = slider(0.1, 10.0, t);
        let blur_lbl = label("");
        let scales_row = row("확대 배율", &[&ocr_scales]);
        let mp_row = row("화소 상한", &[&ocr_mp, &label("백만")]);
        let blur_row = row("흐림 강도", &[&blur, &blur_lbl]);

        // 파일
        let rename = checkbox("파일·폴더 이름의 정보도 비식별화", t, changed);
        let scan_exe = checkbox("실행 파일(exe, dll)도 검사", t, changed);
        let max_mb = num_field(t);
        let min_run = num_field(t);
        let max_row = row("최대 파일 크기", &[&max_mb, &label("MB")]);
        let run_row = row("바이너리 문자열 최소 길이", &[&min_run]);
        let ext_btn = button("검사할 확장자 선택...", t, sel!(showExt:));

        // 사용자 정의 정규식
        let pat_hint = note("한 줄에 하나씩  라벨=정규식   (그룹1이 있으면 그룹1만 치환, #은 주석)\n예) 사번=EMP-\\d{6}");
        let (pat_scroll, patterns) = kit::text_view(true);
        patterns.setDelegate(Some(ProtocolObject::from_ref(ctrl)));
        kit::height(&pat_scroll, 96.0);
        let pat_btns = kit::hstack(&[&button("파일 불러오기...", t, sel!(loadPatterns:)), &button("미리보기에서 시험", t, sel!(testPatterns:))]);
        let pattern_err = note("");
        pattern_err.setTextColor(Some(&NSColor::systemRedColor()));
        let reset = button("모든 설정 기본값으로", t, sel!(resetDefaults:));

        let sections: Vec<(&str, Vec<&NSView>)> = vec![
            ("검출 항목", vec![&types_row, &all_btns]),
            ("탐지 엔진", vec![&ml, &thr_row]),
            ("그림 (OCR)", vec![&ocr, &scales_row, &mp_row, &blur_row]),
            ("파일", vec![&rename, &scan_exe, &max_row, &run_row, &ext_btn]),
            ("사용자 정의 정규식", vec![&pat_hint, &pat_scroll, &pat_btns, &pattern_err]),
        ];
        let heads: Vec<Retained<NSTextField>> = sections.iter().map(|(h, _)| bold_label(h)).collect();
        let mut views: Vec<&NSView> = Vec::new();
        for ((_, items), head) in sections.iter().zip(&heads) {
            views.push(head);
            views.extend(items.iter().copied());
        }
        views.push(&reset);
        let content = kit::vstack(&views);
        for head in heads.iter().skip(1) {
            // 섹션 사이 여백: 제목 바로 앞 항목 뒤에 넓게
            if let Some(i) = views.iter().position(|v| std::ptr::eq(*v, &***head as &NSView)) {
                content.setCustomSpacing_afterView(18.0, views[i - 1]);
            }
        }
        content.setCustomSpacing_afterView(18.0, views[views.len() - 2]);
        // 넓게 채울 것: 정규식 입력·오류·안내
        for v in [&*pat_scroll as &NSView, &pattern_err, &pat_hint] {
            v.widthAnchor().constraintEqualToAnchor(&content.widthAnchor()).setActive(true);
        }
        stretch(&pattern_err);
        let scroll = kit::scroll_panel(&content, 12.0);

        let controls: Vec<Retained<NSControl>> = types
            .iter()
            .map(up)
            .chain([up(&ml), up(&threshold), up(&ocr), up(&rename), up(&scan_exe), up(&max_mb), up(&min_run), up(&ext_btn), up(&reset)])
            .chain([up(&ocr_scales), up(&ocr_mp), up(&blur)])
            .collect();
        let ocr_controls = vec![up(&ocr_scales), up(&ocr_mp), up(&blur)];
        let ui = OptionsUi {
            scroll,
            types,
            ml,
            threshold,
            threshold_lbl,
            ocr,
            ocr_scales,
            ocr_mp,
            blur,
            blur_lbl,
            rename,
            scan_exe,
            max_mb,
            min_run,
            ext_btn,
            patterns,
            pattern_err,
            controls,
            ocr_controls,
        };
        ui.write(&Settings::default());
        ui
    }

    /// 컨트롤 → 설정
    pub fn read(&self, exts: &ExtFilter, input: String, output: String) -> Settings {
        let d = Settings::default();
        let num = |f: &NSTextField| text(f).trim().parse::<f64>().ok();
        Settings {
            input,
            output,
            enabled: std::array::from_fn(|i| is_on(&self.types[i])),
            ml: is_on(&self.ml),
            threshold: ((self.threshold.doubleValue() * 100.0).round() / 100.0) as f32,
            ocr: is_on(&self.ocr),
            ocr_scales: text(&self.ocr_scales),
            ocr_mp: num(&self.ocr_mp).map_or(d.ocr_mp, |v| (v as f32).clamp(1.0, 200.0)),
            ocr_enhance: d.ocr_enhance,
            blur: ((self.blur.doubleValue() * 10.0).round() / 10.0) as f32,
            rename: is_on(&self.rename),
            scan_exe: is_on(&self.scan_exe),
            max_mb: num(&self.max_mb).map_or(d.max_mb, |v| (v as u64).clamp(1, 65536)),
            min_run: num(&self.min_run).map_or(d.min_run, |v| (v as usize).clamp(3, 64)),
            patterns: kit::tv_text(&self.patterns),
            exts: exts.clone(),
        }
    }

    /// 설정 → 컨트롤
    pub fn write(&self, s: &Settings) {
        for (b, on) in self.types.iter().zip(s.enabled) {
            set_on(b, on);
        }
        set_on(&self.ml, s.ml);
        self.threshold.setDoubleValue(s.threshold as f64);
        set_on(&self.ocr, s.ocr);
        self.ocr_scales.setStringValue(&kit::ns(&s.ocr_scales));
        self.ocr_mp.setStringValue(&kit::ns(&format!("{}", s.ocr_mp)));
        self.blur.setDoubleValue(s.blur as f64);
        set_on(&self.rename, s.rename);
        set_on(&self.scan_exe, s.scan_exe);
        self.max_mb.setStringValue(&kit::ns(&s.max_mb.to_string()));
        self.min_run.setStringValue(&kit::ns(&s.min_run.to_string()));
        self.patterns.setString(&kit::ns(&s.patterns));
    }

    pub fn set_all_types(&self, on: bool) {
        self.types.iter().for_each(|b| set_on(b, on));
    }

    /// 값에 딸린 표시 갱신 (슬라이더 숫자, OCR 하위 항목 잠금, 확장자 수, 정규식 오류)
    pub fn sync(&self, exts: &ExtFilter, enabled: bool) {
        self.threshold_lbl.setStringValue(&kit::ns(&format!("{:.2}", self.threshold.doubleValue())));
        self.blur_lbl.setStringValue(&kit::ns(&format!("{:.1}", self.blur.doubleValue())));
        for c in &self.controls {
            c.setEnabled(enabled);
        }
        let ocr_on = enabled && is_on(&self.ocr);
        for c in &self.ocr_controls {
            c.setEnabled(ocr_on);
        }
        self.patterns.setEditable(enabled);
        self.ext_btn.setTitle(&kit::ns(&format!("검사할 확장자 선택 ({}/{})...", exts.enabled.len(), crate::exts::catalog().len())));
        let s = Settings { patterns: kit::tv_text(&self.patterns), ..Default::default() };
        self.pattern_err.setStringValue(&kit::ns(&s.pattern_errors().join("\n")));
    }
}

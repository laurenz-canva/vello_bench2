//! DOM-based UI.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use std::cell::Cell;
use std::rc::Rc;

use crate::backend::{BackendCapabilities, BackendKind};
use crate::scenes::{BenchScene, Param, ParamId, ParamKind};
use crate::storage::UiState;
use wasm_bindgen::{JsCast, prelude::*};
use web_sys::{Document, Element, HtmlElement, HtmlInputElement, HtmlSelectElement};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn doc() -> Document {
    web_sys::window().unwrap().document().unwrap()
}

fn div(d: &Document) -> HtmlElement {
    d.create_element("div").unwrap().dyn_into().unwrap()
}

fn class(el: &impl AsRef<Element>, value: &str) {
    el.as_ref().set_class_name(value);
}

const TOP_PROBE_BUTTON_NEUTRAL_CLASS: &str = "app-probe-button shrink-0 cursor-pointer whitespace-nowrap border border-slate-300/20 bg-slate-300/10 px-2 py-1 text-xs font-semibold text-slate-200 transition hover:bg-slate-300/15";
const TOP_PROBE_BUTTON_SUCCESS_CLASS: &str = "app-probe-button shrink-0 cursor-pointer whitespace-nowrap border border-emerald-300/40 bg-emerald-300/10 px-2 py-1 text-xs font-semibold text-emerald-300 transition hover:bg-emerald-300/15";
const TOP_PROBE_BUTTON_FAILURE_CLASS: &str = "app-probe-button shrink-0 cursor-pointer whitespace-nowrap border border-rose-300/40 bg-rose-300/10 px-2 py-1 text-xs font-semibold text-rose-300 transition hover:bg-rose-300/15";
const PROBE_DETAILS_NEUTRAL_CLASS: &str =
    "app-probe-details border border-slate-300/20 bg-slate-300/10 text-slate-200";
const PROBE_DETAILS_SUCCESS_CLASS: &str =
    "app-probe-details border border-emerald-300/40 bg-emerald-300/10 text-emerald-300";
const PROBE_DETAILS_FAILURE_CLASS: &str =
    "app-probe-details border border-rose-300/40 bg-rose-300/10 text-rose-300";

fn set_probe_button_state(button: &HtmlElement, class_name: &str, text: &str, title: Option<&str>) {
    class(button, class_name);
    button.set_text_content(Some(text));
    match title {
        Some(title) => button.set_attribute("title", title).unwrap(),
        None => button.remove_attribute("title").unwrap(),
    }
}

fn set_probe_details(details: &HtmlElement, class_name: &str, text: Option<&str>) {
    class(details, class_name);
    details.set_text_content(text);
    details
        .style()
        .set_property("display", if text.is_some() { "block" } else { "none" })
        .unwrap();
}

fn select_style(sel: &HtmlSelectElement) {
    class(
        sel,
        "w-full rounded-xl border border-white/10 bg-slate-950/80 px-3 py-2 text-sm text-slate-100 outline-none transition focus:border-cyan-300/60 focus:ring-2 focus:ring-cyan-300/20",
    );
}

fn format_val(v: f64, step: f64) -> String {
    if step >= 1.0 || v.fract().abs() < f64::EPSILON {
        format!("{}", v as i64)
    } else {
        format!("{v:.1}")
    }
}

fn range_step(value: f64, base_step: f64) -> f64 {
    if value.abs() < 1.0 {
        return if base_step < 1.0 { base_step } else { 1.0 };
    }
    10f64.powf(value.abs().log10().floor())
}

fn snap_to_step(value: f64, base_step: f64) -> f64 {
    if value.abs() < 1.0 && base_step < 1.0 {
        return (value / base_step).round() * base_step;
    }
    let step = range_step(value, base_step);
    (value / step).round() * step
}

fn stepper_delta(value: f64, base_step: f64) -> f64 {
    range_step(value, base_step)
}

fn stepper_decrement(value: f64, base_step: f64) -> f64 {
    let step = range_step(value, base_step);
    if step > base_step && value.abs() >= 10.0 && (value.abs() - step).abs() < f64::EPSILON {
        (step / 10.0).max(base_step)
    } else {
        step
    }
}

fn set_stepper_value(input: &HtmlInputElement, label: &HtmlElement, value: f64, step: f64) {
    let snapped = snap_to_step(value, step);
    input.set_value(&snapped.to_string());
    label.set_text_content(Some(&format_val(snapped, range_step(snapped, step))));
}

fn sanitized_stepper_value(input: &HtmlInputElement, label: &HtmlElement, step: f64) -> f64 {
    let raw = label.text_content().unwrap_or_default();
    let trimmed = raw.trim();
    if let Ok(value) = trimmed.parse::<f64>() {
        input.set_value(trimmed);
        value
    } else {
        let fallback = input.value().parse().unwrap_or(0.0);
        label.set_text_content(Some(&format_val(fallback, range_step(fallback, step))));
        fallback
    }
}

// ── Param control ────────────────────────────────────────────────────────────

enum ParamCtrl {
    Stepper {
        root: HtmlElement,
        input: HtmlInputElement,
        step: f64,
    },
    Select {
        root: HtmlElement,
        select: HtmlSelectElement,
    },
}

impl ParamCtrl {
    fn root(&self) -> &HtmlElement {
        match self {
            Self::Stepper { root, .. } | Self::Select { root, .. } => root,
        }
    }
}

// ── UI ───────────────────────────────────────────────────────────────────────

/// Full UI state.
pub struct Ui {
    // Layout
    #[allow(dead_code, reason = "kept alive to prevent GC")]
    top_bar: HtmlElement,
    // Top bar
    top_timing_label: HtmlElement,
    top_timing_popup: HtmlElement,
    webgl_init_status: HtmlElement,
    top_probe_btn: HtmlElement,
    top_probe_details: HtmlElement,
    renderer_select: HtmlSelectElement,

    // Interactive: sidebar
    sidebar: HtmlElement,
    toggle_btn: HtmlElement,
    sidebar_collapsed: bool,
    viewport_label: HtmlElement,
    /// Scene selector.
    pub scene_select: HtmlSelectElement,
    controls: Vec<(ParamCtrl, HtmlElement, ParamId)>,
    /// Cached relevance mask, avoiding DOM writes when configuration is unchanged.
    param_visibility_mask: Cell<Option<u64>>,
    /// Reset view button.
    pub reset_view_btn: HtmlElement,

    /// Whether state needs saving to localStorage.
    /// Shared with closures that mark it on param/checkbox changes.
    dirty: Rc<Cell<bool>>,
}

impl std::fmt::Debug for Ui {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ui").finish_non_exhaustive()
    }
}

impl Ui {
    /// Build the entire UI.
    pub(crate) fn build(
        document: &Document,
        scenes: &[Box<dyn BenchScene>],
        capabilities: BackendCapabilities,
        current_scene: usize,
        sidebar_collapsed: bool,
        vp_w: u32,
        vp_h: u32,
    ) -> Self {
        let body = document.body().unwrap();
        class(&body, "overflow-hidden antialiased");
        let app_overlay = document
            .get_element_by_id("app-overlay")
            .expect("app-overlay should exist in index.html");
        let dirty = Rc::new(Cell::new(false));

        let (
            top_bar,
            sidebar_toggle_btn,
            webgl_init_status,
            top_probe_btn,
            top_probe_details,
            renderer_select,
        ) = build_top_bar(document, crate::backend::current_backend_kind());
        app_overlay.append_child(&top_bar).unwrap();

        let iv = build_interactive_view(
            document,
            scenes,
            capabilities,
            current_scene,
            vp_w,
            vp_h,
            &dirty,
        );
        app_overlay.append_child(&iv.view).unwrap();

        let ui = Self {
            top_bar,
            top_timing_label: iv.top_timing_label,
            top_timing_popup: iv.top_timing_popup,
            webgl_init_status,
            top_probe_btn,
            top_probe_details,
            renderer_select,
            sidebar: iv.sidebar,
            toggle_btn: sidebar_toggle_btn,
            sidebar_collapsed,
            viewport_label: iv.viewport_label,
            scene_select: iv.scene_select,
            controls: iv.controls,
            param_visibility_mask: Cell::new(None),
            reset_view_btn: iv.reset_view_btn,
            dirty,
        };
        ui.set_renderer(crate::backend::current_backend_kind());
        ui.apply_sidebar_state();
        let values = ui.read_params();
        ui.sync_param_visibility(scenes[current_scene].scene_id(), &values);
        ui
    }

    pub fn renderer_select(&self) -> &HtmlSelectElement {
        &self.renderer_select
    }

    pub fn set_renderer(&self, kind: BackendKind) {
        self.renderer_select.set_value(kind.as_str());
        self.sync_probe_button(kind);
    }

    pub fn set_webgl_initializing(&self) {
        self.webgl_init_status
            .set_text_content(Some("Initializing WebGL…"));
        class(
            &self.webgl_init_status,
            "webgl-init-status webgl-init-status-pending",
        );
    }

    pub fn set_webgl_initialized(&self) {
        self.webgl_init_status
            .set_text_content(Some("WebGL initialization succeeded"));
        class(
            &self.webgl_init_status,
            "webgl-init-status webgl-init-status-success",
        );
    }

    pub fn hide_webgl_init_status(&self) {
        class(&self.webgl_init_status, "webgl-init-status");
    }

    // ── Sidebar toggle ───────────────────────────────────────────────────

    /// Toggle sidebar.
    pub fn toggle_sidebar(&mut self) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        self.apply_sidebar_state();
        self.dirty.set(true);
    }

    /// Toggle button for event binding.
    pub fn toggle_btn(&self) -> &HtmlElement {
        &self.toggle_btn
    }

    /// Sidebar element (for hit-testing).
    pub fn sidebar(&self) -> &HtmlElement {
        &self.sidebar
    }

    fn apply_sidebar_state(&self) {
        let transform = if self.sidebar_collapsed {
            "translateX(-100%)"
        } else {
            "translateX(0)"
        };
        self.sidebar
            .style()
            .set_property("transform", transform)
            .unwrap();
    }

    // ── Interactive displays ─────────────────────────────────────────────

    /// Update FPS/render displays.
    pub fn update_timing(
        &self,
        fps: f64,
        frame_time: f64,
        encode_ms: f64,
        render_ms: f64,
        blit_ms: f64,
        total_ms: f64,
        is_cpu: bool,
        supports_encode_timing: bool,
    ) {
        self.top_timing_label
            .set_text_content(Some(&format!("FPS: {fps:.1} ({frame_time:.1}ms)")));
        let encode = if supports_encode_timing {
            format!("{encode_ms:.2}ms")
        } else {
            "--".to_string()
        };
        let render = if is_cpu {
            format!("{render_ms:.2}ms")
        } else {
            "--".to_string()
        };
        let blit = if is_cpu {
            format!("{blit_ms:.2}ms")
        } else {
            "--".to_string()
        };
        let total = if is_cpu {
            format!("{total_ms:.2}ms")
        } else {
            "--".to_string()
        };
        self.top_timing_popup.set_inner_html(&format!(
            "<div class=\"space-y-1 text-slate-600\"><div>Encode: {encode}</div><div>Render: {render}</div><div>Blit: {blit}</div><div>Total: {total}</div></div>"
        ));
    }

    /// Update viewport display.
    pub fn update_viewport(&self, w: u32, h: u32) {
        self.viewport_label
            .set_text_content(Some(&format!("Viewport: {w} x {h}")));
    }

    /// Read interactive param values.
    pub fn read_params(&self) -> Vec<(ParamId, f64)> {
        self.controls
            .iter()
            .map(|(ctrl, val_span, param_id)| {
                let v: f64 = match ctrl {
                    ParamCtrl::Stepper { input, step, .. } => {
                        sanitized_stepper_value(input, val_span, *step)
                    }
                    ParamCtrl::Select { select, .. } => select.value().parse().unwrap_or(0.0),
                };
                (*param_id, v)
            })
            .collect()
    }

    /// Show only controls that affect the scene's current configuration.
    pub fn sync_param_visibility(
        &self,
        scene_id: crate::scenes::SceneId,
        values: &[(ParamId, f64)],
    ) {
        let visibility_mask = self.controls.iter().fold(0_u64, |mask, (_, _, param_id)| {
            if crate::scenes::param_is_relevant(scene_id, *param_id, values) {
                mask | param_id.bit()
            } else {
                mask
            }
        });
        if self.param_visibility_mask.get() == Some(visibility_mask) {
            return;
        }

        for (ctrl, _, param_id) in &self.controls {
            let display = if visibility_mask & param_id.bit() != 0 {
                ""
            } else {
                "none"
            };
            ctrl.root()
                .style()
                .set_property("display", display)
                .unwrap();
        }
        self.param_visibility_mask.set(Some(visibility_mask));
    }

    /// Rebuild interactive params.
    pub fn rebuild_params(&mut self, params: &[Param]) {
        for (ctrl, _, _) in self.controls.drain(..) {
            match ctrl {
                ParamCtrl::Stepper { root, .. } => root.remove(),
                ParamCtrl::Select { root, .. } => root.remove(),
            }
        }
        self.param_visibility_mask.set(None);
        let document = doc();
        self.controls = build_controls(&document, &self.sidebar, params, None, Some(&self.dirty));
    }

    pub fn rebuild_scene_options(
        &self,
        scenes: &[Box<dyn BenchScene>],
        capabilities: BackendCapabilities,
        current_scene: usize,
    ) {
        while let Some(child) = self.scene_select.first_child() {
            self.scene_select.remove_child(&child).unwrap();
        }
        let document = doc();
        for (i, s) in scenes.iter().enumerate() {
            let opt = document.create_element("option").unwrap();
            opt.set_text_content(Some(s.name()));
            opt.set_attribute("value", &i.to_string()).unwrap();
            if !capabilities.supports_scene(s.scene_id()) {
                opt.set_attribute("hidden", "true").unwrap();
                opt.set_attribute("disabled", "true").unwrap();
            }
            self.scene_select.append_child(&opt).unwrap();
        }
        self.scene_select.set_value(&current_scene.to_string());
    }

    /// Selected interactive scene index.
    pub fn selected_scene(&self) -> usize {
        self.scene_select.value().parse().unwrap_or(0)
    }

    pub fn top_probe_btn(&self) -> &HtmlElement {
        &self.top_probe_btn
    }

    pub fn set_probe_running(&self, running: bool) {
        self.top_probe_btn
            .style()
            .set_property("opacity", if running { "0.7" } else { "1" })
            .unwrap();
        self.top_probe_btn
            .style()
            .set_property("pointer-events", if running { "none" } else { "auto" })
            .unwrap();
        if running {
            set_probe_button_state(
                &self.top_probe_btn,
                TOP_PROBE_BUTTON_NEUTRAL_CLASS,
                "Probing...",
                None,
            );
            set_probe_details(
                &self.top_probe_details,
                PROBE_DETAILS_NEUTRAL_CLASS,
                Some("Starting probe…"),
            );
        } else {
            set_probe_button_state(
                &self.top_probe_btn,
                TOP_PROBE_BUTTON_NEUTRAL_CLASS,
                "Probe",
                None,
            );
            set_probe_details(&self.top_probe_details, PROBE_DETAILS_NEUTRAL_CLASS, None);
        }
    }

    pub fn set_probe_sync_complete(&self, synchronous_ms: f64) {
        set_probe_button_state(
            &self.top_probe_btn,
            TOP_PROBE_BUTTON_NEUTRAL_CLASS,
            "Probing…",
            None,
        );
        set_probe_details(
            &self.top_probe_details,
            PROBE_DETAILS_NEUTRAL_CLASS,
            Some(&format!(
                "start_probe: {synchronous_ms:.1}ms · readback: pending · full: pending"
            )),
        );
    }

    pub fn set_probe_success(&self, start_probe_ms: f64, readback_ms: f64, full_ms: f64) {
        self.top_probe_btn
            .style()
            .set_property("opacity", "1")
            .unwrap();
        self.top_probe_btn
            .style()
            .set_property("pointer-events", "auto")
            .unwrap();
        set_probe_button_state(
            &self.top_probe_btn,
            TOP_PROBE_BUTTON_SUCCESS_CLASS,
            "Probe: Pass",
            None,
        );
        set_probe_details(
            &self.top_probe_details,
            PROBE_DETAILS_SUCCESS_CLASS,
            Some(&format!(
                "start_probe: {start_probe_ms:.1}ms · readback: {readback_ms:.1}ms · full: {full_ms:.1}ms"
            )),
        );
    }

    pub fn set_probe_failure(
        &self,
        message: &str,
        start_probe_ms: f64,
        readback_ms: Option<f64>,
        full_ms: f64,
    ) {
        self.top_probe_btn
            .style()
            .set_property("opacity", "1")
            .unwrap();
        self.top_probe_btn
            .style()
            .set_property("pointer-events", "auto")
            .unwrap();
        set_probe_button_state(
            &self.top_probe_btn,
            TOP_PROBE_BUTTON_FAILURE_CLASS,
            "Probe: Error",
            None,
        );
        set_probe_details(
            &self.top_probe_details,
            PROBE_DETAILS_FAILURE_CLASS,
            Some(&format!(
                "start_probe: {start_probe_ms:.1}ms · readback: {} · full: {full_ms:.1}ms · error: {message}",
                readback_ms
                    .map(|value| format!("{value:.1}ms"))
                    .unwrap_or_else(|| "n/a".to_string())
            )),
        );
    }

    fn sync_probe_button(&self, kind: BackendKind) {
        let visible = kind == BackendKind::Hybrid;
        self.top_probe_btn
            .style()
            .set_property("display", if visible { "block" } else { "none" })
            .unwrap();
        self.top_probe_btn
            .style()
            .set_property("opacity", "1")
            .unwrap();
        self.top_probe_btn
            .style()
            .set_property("pointer-events", "auto")
            .unwrap();
        set_probe_button_state(
            &self.top_probe_btn,
            TOP_PROBE_BUTTON_NEUTRAL_CLASS,
            "Probe",
            None,
        );
        set_probe_details(&self.top_probe_details, PROBE_DETAILS_NEUTRAL_CLASS, None);
    }

    /// Mark state as needing a save.
    pub fn mark_dirty(&self) {
        self.dirty.set(true);
    }

    /// If dirty, persist current state to localStorage.
    pub fn flush_state(&self) {
        if self.dirty.get() {
            self.dirty.set(false);
            self.save_state();
        }
    }

    /// Return a clone of the dirty flag for use in closures.
    pub fn dirty_flag(&self) -> Rc<Cell<bool>> {
        self.dirty.clone()
    }

    /// Write current UI state to localStorage.
    pub fn save_state(&self) {
        let scene = self.selected_scene();
        let params: Vec<(String, f64)> = self
            .controls
            .iter()
            .map(|(ctrl, val_span, param_id)| {
                let v: f64 = match ctrl {
                    ParamCtrl::Stepper { input, step, .. } => {
                        sanitized_stepper_value(input, val_span, *step)
                    }
                    ParamCtrl::Select { select, .. } => select.value().parse().unwrap_or(0.0),
                };
                (param_id.as_str().to_string(), v)
            })
            .collect();
        crate::storage::save_ui_state(&UiState {
            sidebar_collapsed: Some(self.sidebar_collapsed),
            scene: Some(scene),
            params,
        });
    }

    /// Apply saved interactive param values.
    pub(crate) fn apply_saved_params(&self, saved: &UiState) {
        for (ctrl, val_span, param_id) in &self.controls {
            if let Some((_, v)) = saved.params.iter().find(|(k, _)| k == param_id.as_str()) {
                match ctrl {
                    ParamCtrl::Stepper { input, step, .. } => {
                        set_stepper_value(input, val_span, *v, *step);
                    }
                    ParamCtrl::Select { select, .. } => {
                        select.set_value(&v.to_string());
                    }
                }
            }
        }
    }
}

struct InteractiveViewParts {
    view: HtmlElement,
    sidebar: HtmlElement,
    top_timing_label: HtmlElement,
    top_timing_popup: HtmlElement,
    viewport_label: HtmlElement,
    scene_select: HtmlSelectElement,
    controls: Vec<(ParamCtrl, HtmlElement, ParamId)>,
    reset_view_btn: HtmlElement,
}

// ── Sub-builders ─────────────────────────────────────────────────────────────

fn build_top_bar(
    document: &Document,
    current_backend: BackendKind,
) -> (
    HtmlElement,
    HtmlElement,
    HtmlElement,
    HtmlElement,
    HtmlElement,
    HtmlSelectElement,
) {
    let top_bar = div(document);
    class(&top_bar, "app-top-bar");

    let nav_group = div(document);
    class(&nav_group, "app-nav-control");

    let sidebar_toggle_btn = div(document);
    sidebar_toggle_btn.set_inner_html(
        "<div class=\"flex flex-col gap-1\"><span class=\"block h-px w-4 bg-slate-100\"></span><span class=\"block h-px w-4 bg-slate-100\"></span><span class=\"block h-px w-4 bg-slate-100\"></span></div>",
    );
    class(
        &sidebar_toggle_btn,
        "flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center border border-white/10 bg-slate-900/80 text-slate-100 hover:bg-slate-900",
    );
    nav_group.append_child(&sidebar_toggle_btn).unwrap();

    top_bar.append_child(&nav_group).unwrap();

    let webgl_init_status = div(document);
    class(&webgl_init_status, "webgl-init-status");

    let controls_group = div(document);
    class(&controls_group, "app-top-controls");

    let primary_controls = div(document);
    class(&primary_controls, "app-top-primary");
    controls_group.append_child(&primary_controls).unwrap();
    controls_group.append_child(&webgl_init_status).unwrap();

    let has_toggle = js_sys::Reflect::get(&js_sys::global(), &"__vello_toggle_simd".into())
        .ok()
        .map_or(false, |v| v.is_function());
    if has_toggle {
        let simd_on = js_sys::Reflect::get(&js_sys::global(), &"__vello_simd".into())
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let simd_btn = div(document);
        simd_btn.set_text_content(Some(if simd_on { "SIMD: ON" } else { "SIMD: OFF" }));
        class(
            &simd_btn,
            if simd_on {
                "app-simd-toggle shrink-0 cursor-pointer whitespace-nowrap border border-emerald-300/40 bg-emerald-300/10 px-2 py-1 text-xs font-semibold text-emerald-300"
            } else {
                "app-simd-toggle shrink-0 cursor-pointer whitespace-nowrap border border-rose-300/40 bg-rose-300/10 px-2 py-1 text-xs font-semibold text-rose-300"
            },
        );
        {
            let cb = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
                if let Ok(f) =
                    js_sys::Reflect::get(&js_sys::global(), &"__vello_toggle_simd".into())
                {
                    if let Some(f) = f.dyn_ref::<js_sys::Function>() {
                        let _ = f.call0(&wasm_bindgen::JsValue::NULL);
                    }
                }
            }) as Box<dyn FnMut()>);
            simd_btn
                .add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
                .unwrap();
            cb.forget();
        }
        primary_controls.append_child(&simd_btn).unwrap();
    }

    let top_probe_btn = div(document);
    set_probe_button_state(
        &top_probe_btn,
        TOP_PROBE_BUTTON_NEUTRAL_CLASS,
        "Probe",
        None,
    );
    top_probe_btn
        .style()
        .set_property("display", "none")
        .unwrap();
    let top_probe_details = div(document);
    set_probe_details(&top_probe_details, PROBE_DETAILS_NEUTRAL_CLASS, None);
    controls_group.append_child(&top_probe_details).unwrap();

    let renderer_select: HtmlSelectElement = document
        .create_element("select")
        .unwrap()
        .dyn_into()
        .unwrap();
    select_style(&renderer_select);
    class(
        &renderer_select,
        "app-renderer-select w-auto max-w-[9rem] shrink border border-white/10 bg-slate-950/80 px-3 py-1 text-sm text-slate-100",
    );
    for kind in BackendKind::available() {
        let opt = document.create_element("option").unwrap();
        opt.set_text_content(Some(kind.label()));
        opt.set_attribute("value", kind.as_str()).unwrap();
        renderer_select.append_child(&opt).unwrap();
    }
    renderer_select.set_value(current_backend.as_str());
    primary_controls.append_child(&renderer_select).unwrap();
    primary_controls.append_child(&top_probe_btn).unwrap();
    top_bar.append_child(&controls_group).unwrap();

    (
        top_bar,
        sidebar_toggle_btn,
        webgl_init_status,
        top_probe_btn,
        top_probe_details,
        renderer_select,
    )
}

fn build_interactive_view(
    document: &Document,
    scenes: &[Box<dyn BenchScene>],
    capabilities: BackendCapabilities,
    current_scene: usize,
    vp_w: u32,
    vp_h: u32,
    dirty: &Rc<Cell<bool>>,
) -> InteractiveViewParts {
    let view = div(document);
    class(&view, "fixed inset-0 z-20 pointer-events-none");

    let (top_timing_wrap, top_timing_label, top_timing_popup) = build_timing_overlay(document);
    view.append_child(&top_timing_wrap).unwrap();

    let sidebar = div(document);
    class(
        &sidebar,
        "sidebar-scroll pointer-events-auto fixed bottom-0 left-0 top-28 z-20 flex w-[240px] flex-col overflow-y-auto border-r border-white/10 bg-slate-950/58 px-3 py-4 transition-transform duration-200 sm:top-16 lg:top-20 lg:w-[220px]",
    );

    let viewport_label = div(document);
    viewport_label.set_text_content(Some(&format!("Viewport: {vp_w} x {vp_h}")));
    class(&viewport_label, "mb-3 px-1 text-[11px] text-slate-400");
    sidebar.append_child(&viewport_label).unwrap();

    let lbl = div(document);
    lbl.set_text_content(Some("Scene"));
    class(
        &lbl,
        "mb-2 text-[0.65rem] font-semibold uppercase tracking-[0.32em] text-slate-400",
    );
    sidebar.append_child(&lbl).unwrap();

    let scene_select: HtmlSelectElement = document
        .create_element("select")
        .unwrap()
        .dyn_into()
        .unwrap();
    select_style(&scene_select);
    class(
        &scene_select,
        "mb-2 w-full border border-white/10 bg-slate-950/80 px-2 py-1.5 text-sm text-slate-100",
    );
    for (i, s) in scenes.iter().enumerate() {
        let opt = document.create_element("option").unwrap();
        opt.set_text_content(Some(s.name()));
        opt.set_attribute("value", &i.to_string()).unwrap();
        if !capabilities.supports_scene(s.scene_id()) {
            opt.set_attribute("hidden", "true").unwrap();
            opt.set_attribute("disabled", "true").unwrap();
        }
        scene_select.append_child(&opt).unwrap();
    }
    scene_select.set_value(&current_scene.to_string());
    sidebar.append_child(&scene_select).unwrap();

    let sep = div(document);
    class(&sep, "my-1 border-t border-white/10");
    sidebar.append_child(&sep).unwrap();

    let controls = build_controls(
        document,
        &sidebar,
        &crate::scenes::visible_params(scenes[current_scene].as_ref(), capabilities),
        None,
        Some(dirty),
    );

    let reset_view_btn = div(document);
    reset_view_btn.set_inner_html("<span class=\"text-sm leading-none\">&#10226;</span>");
    class(
        &reset_view_btn,
        "pointer-events-auto fixed bottom-4 right-4 z-[75] hidden h-10 w-10 items-center justify-center border border-white/10 bg-slate-950/88 text-slate-100 transition hover:border-cyan-300/40 hover:bg-slate-900/95",
    );
    view.append_child(&reset_view_btn).unwrap();

    view.append_child(&sidebar).unwrap();

    InteractiveViewParts {
        view,
        sidebar,
        top_timing_label,
        top_timing_popup,
        viewport_label,
        scene_select,
        controls,
        reset_view_btn,
    }
}

fn build_timing_overlay(document: &Document) -> (HtmlElement, HtmlElement, HtmlElement) {
    let timing_wrap = div(document);
    class(
        &timing_wrap,
        "pointer-events-auto fixed right-3 top-[7.5rem] z-[70] flex items-start sm:top-[4.5rem] lg:right-4 lg:top-24",
    );

    let top_timing_label = div(document);
    top_timing_label.set_text_content(Some("-- FPS  -- ms/f"));
    class(
        &top_timing_label,
        "whitespace-nowrap border border-white/10 bg-slate-950/88 px-3 py-2 text-xs font-semibold text-emerald-300",
    );

    let top_timing_popup = div(document);
    top_timing_popup.set_inner_html(
        "<div class=\"space-y-1 text-slate-300\"><div>Encode: --</div><div>Render: --</div><div>Blit: --</div><div>Total: --</div></div>",
    );
    class(
        &top_timing_popup,
        "pointer-events-none absolute right-0 top-full z-[90] mt-2 hidden min-w-[12rem] border border-white/10 bg-slate-950 px-4 py-3 text-xs text-slate-300",
    );
    {
        let popup = top_timing_popup.clone();
        let timeout_id = Rc::new(Cell::new(None::<i32>));
        let enter_timeout = timeout_id.clone();
        let enter = Closure::wrap(Box::new(move || {
            let popup = popup.clone();
            let cb = Closure::once_into_js(move || {
                let _ = popup.style().set_property("display", "block");
            });
            let id = web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    300,
                )
                .unwrap();
            enter_timeout.set(Some(id));
        }) as Box<dyn FnMut()>);
        timing_wrap
            .add_event_listener_with_callback("mouseenter", enter.as_ref().unchecked_ref())
            .unwrap();
        enter.forget();

        let popup = top_timing_popup.clone();
        let leave_timeout = timeout_id.clone();
        let leave = Closure::wrap(Box::new(move || {
            if let Some(id) = leave_timeout.get() {
                web_sys::window().unwrap().clear_timeout_with_handle(id);
                leave_timeout.set(None);
            }
            let _ = popup.style().set_property("display", "none");
        }) as Box<dyn FnMut()>);
        timing_wrap
            .add_event_listener_with_callback("mouseleave", leave.as_ref().unchecked_ref())
            .unwrap();
        leave.forget();
    }

    timing_wrap.append_child(&top_timing_label).unwrap();
    timing_wrap.append_child(&top_timing_popup).unwrap();
    (timing_wrap, top_timing_label, top_timing_popup)
}

fn build_controls(
    document: &Document,
    container: &Element,
    params: &[Param],
    insert_before: Option<&HtmlElement>,
    dirty: Option<&Rc<Cell<bool>>>,
) -> Vec<(ParamCtrl, HtmlElement, ParamId)> {
    let mut out = Vec::new();

    for p in params {
        let row = div(document);
        class(&row, "mb-3 border-b border-white/10 pb-3");

        let label = div(document);
        label.set_text_content(Some(p.label));
        class(
            &label,
            "mb-2 text-[0.65rem] font-semibold uppercase tracking-[0.32em] text-slate-400",
        );
        row.append_child(&label).unwrap();

        let val_span = div(document);
        class(&val_span, "ml-2 inline text-slate-100");

        let ctrl = match &p.kind {
            ParamKind::Slider {
                min: _,
                max: _,
                step,
            } => {
                let input: HtmlInputElement = document
                    .create_element("input")
                    .unwrap()
                    .dyn_into()
                    .unwrap();
                input.set_type("hidden");
                input.set_value(&p.value.to_string());
                row.append_child(&input).unwrap();

                let stepper = div(document);
                class(&stepper, "flex items-center gap-2");

                let button_class = "flex h-8 w-8 shrink-0 items-center justify-center border border-white/10 bg-slate-950/85 text-base leading-none text-slate-100 transition hover:border-cyan-300/40 hover:bg-slate-900";

                let minus = div(document);
                minus.set_text_content(Some("-"));
                class(&minus, button_class);
                stepper.append_child(&minus).unwrap();

                class(
                    &val_span,
                    "flex min-h-8 flex-1 items-center justify-center overflow-hidden border border-white/10 bg-slate-950/78 px-2 text-sm font-medium text-slate-100 outline-none",
                );
                val_span.set_attribute("contenteditable", "true").unwrap();
                val_span.set_attribute("spellcheck", "false").unwrap();
                val_span.set_attribute("tabindex", "0").unwrap();
                set_stepper_value(&input, &val_span, p.value, *step);
                stepper.append_child(&val_span).unwrap();

                let plus = div(document);
                plus.set_text_content(Some("+"));
                class(&plus, button_class);
                stepper.append_child(&plus).unwrap();
                row.append_child(&stepper).unwrap();

                let minus_input = input.clone();
                let minus_label = val_span.clone();
                let minus_dirty = dirty.cloned();
                let base_step = *step;
                let initial_value = p.value;
                let minus_cb = Closure::wrap(Box::new(move || {
                    let current = minus_input.value().parse().unwrap_or(initial_value);
                    let next = current - stepper_decrement(current, base_step);
                    set_stepper_value(&minus_input, &minus_label, next, base_step);
                    if let Some(ref d) = minus_dirty {
                        d.set(true);
                    }
                }) as Box<dyn FnMut()>);
                minus
                    .add_event_listener_with_callback("click", minus_cb.as_ref().unchecked_ref())
                    .unwrap();
                minus_cb.forget();

                let plus_input = input.clone();
                let plus_label = val_span.clone();
                let plus_dirty = dirty.cloned();
                let plus_cb = Closure::wrap(Box::new(move || {
                    let current = plus_input.value().parse().unwrap_or(initial_value);
                    let next = current + stepper_delta(current, base_step);
                    set_stepper_value(&plus_input, &plus_label, next, base_step);
                    if let Some(ref d) = plus_dirty {
                        d.set(true);
                    }
                }) as Box<dyn FnMut()>);
                plus.add_event_listener_with_callback("click", plus_cb.as_ref().unchecked_ref())
                    .unwrap();
                plus_cb.forget();

                if let Some(edit_dirty) = dirty.cloned() {
                    let edit_cb = Closure::wrap(Box::new(move || {
                        edit_dirty.set(true);
                    }) as Box<dyn FnMut()>);
                    val_span
                        .add_event_listener_with_callback("input", edit_cb.as_ref().unchecked_ref())
                        .unwrap();
                    edit_cb.forget();
                }

                let key_input = input.clone();
                let key_label = val_span.clone();
                let key_step = *step;
                let key_cb = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
                    if event.key() == "Enter" {
                        event.prevent_default();
                        let _ = sanitized_stepper_value(&key_input, &key_label, key_step);
                        let _ = key_label.blur();
                    }
                }) as Box<dyn FnMut(_)>);
                val_span
                    .add_event_listener_with_callback("keydown", key_cb.as_ref().unchecked_ref())
                    .unwrap();
                key_cb.forget();

                ParamCtrl::Stepper {
                    root: row.clone(),
                    input,
                    step: *step,
                }
            }
            ParamKind::Select(options) => {
                let sel: HtmlSelectElement = document
                    .create_element("select")
                    .unwrap()
                    .dyn_into()
                    .unwrap();
                select_style(&sel);
                class(
                    &sel,
                    "w-full border border-white/10 bg-slate-950/78 px-2 py-1.5 text-sm text-slate-100 outline-none focus:border-cyan-300/60 focus:ring-2 focus:ring-cyan-300/20",
                );
                for &(text, val) in options {
                    let opt = document.create_element("option").unwrap();
                    opt.set_text_content(Some(text));
                    opt.set_attribute("value", &val.to_string()).unwrap();
                    sel.append_child(&opt).unwrap();
                }
                let idx = options
                    .iter()
                    .position(|&(_, v)| (v - p.value).abs() < f64::EPSILON)
                    .unwrap_or(0);
                sel.set_selected_index(idx as i32);
                row.append_child(&sel).unwrap();

                if let Some(dirty) = dirty.cloned() {
                    let cb = Closure::wrap(Box::new(move || {
                        dirty.set(true);
                    }) as Box<dyn FnMut()>);
                    sel.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
                        .unwrap();
                    cb.forget();
                }

                ParamCtrl::Select {
                    root: row.clone(),
                    select: sel,
                }
            }
        };

        if let Some(before) = insert_before {
            container.insert_before(&row, Some(before)).unwrap();
        } else {
            container.append_child(&row).unwrap();
        }
        out.push((ctrl, val_span, p.id));
    }

    out
}

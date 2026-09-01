//! Interactive renderer playground for Vello Hybrid.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]
#![cfg(target_arch = "wasm32")]

pub(crate) mod backend;
pub(crate) mod capability;
mod fps;
pub(crate) mod resource_store;
pub(crate) mod rng;
pub mod scenes;
pub(crate) mod storage;
pub mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use backend::{
    Backend, BackendCapabilities, BackendKind, current_backend_capabilities, current_backend_kind,
    new_backend,
};
use fps::FpsTracker;
use resource_store::ResourceStore;
use scenes::{BenchScene, scene_index};
use ui::Ui;
use vello_common::kurbo::Affine;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::HtmlCanvasElement;

type RafClosure = Rc<RefCell<Option<Closure<dyn FnMut()>>>>;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = requestAnimationFrame)]
    fn request_animation_frame(f: &Closure<dyn FnMut()>);
}

pub fn init_logging() {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);
}

fn probe_elapsed_ms(started_at: f64) -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now() - started_at)
        .unwrap_or(0.0)
}

async fn next_animation_frame() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .request_animation_frame(resolve.unchecked_ref())
            .unwrap();
    });
    JsFuture::from(promise).await.unwrap();
}

fn poll_probe_completion<F>(pending_probe: vello_hybrid::WebGlPendingProbe, on_complete: F)
where
    F: FnOnce(ProbeCompletion) + 'static,
{
    let callback: RafClosure = Rc::new(RefCell::new(None));
    let callback_ref = callback.clone();
    let pending_probe = Rc::new(RefCell::new(Some(pending_probe)));
    let pending_probe_ref = pending_probe.clone();
    let on_complete = Rc::new(RefCell::new(Some(on_complete)));
    let on_complete_ref = on_complete.clone();

    *callback.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        let Some(pending_probe) = pending_probe_ref.borrow_mut().take() else {
            return;
        };

        let readback_started_at = web_sys::window()
            .and_then(|window| window.performance())
            .map(|performance| performance.now())
            .unwrap_or(0.0);
        match pending_probe.try_finish() {
            Ok(vello_hybrid::WebGlProbeStatus::Pending(pending_probe)) => {
                *pending_probe_ref.borrow_mut() = Some(pending_probe);
                if let Some(callback) = callback_ref.borrow().as_ref() {
                    request_animation_frame(callback);
                }
            }
            Ok(vello_hybrid::WebGlProbeStatus::Complete(probe)) => {
                let readback_ms = probe_elapsed_ms(readback_started_at);
                callback_ref.borrow_mut().take();
                if let Some(on_complete) = on_complete_ref.borrow_mut().take() {
                    on_complete(ProbeCompletion {
                        readback_ms,
                        result: probe_result_to_result(probe),
                    });
                }
            }
            Err(error) => {
                let readback_ms = probe_elapsed_ms(readback_started_at);
                callback_ref.borrow_mut().take();
                if let Some(on_complete) = on_complete_ref.borrow_mut().take() {
                    on_complete(ProbeCompletion {
                        readback_ms,
                        result: Err(error.to_string()),
                    });
                }
            }
        }
    }) as Box<dyn FnMut()>));

    if let Some(callback) = callback.borrow().as_ref() {
        request_animation_frame(callback);
    }
}

fn probe_result_to_result(
    probe: vello_common::probe::Probe<vello_hybrid::RenderError>,
) -> Result<(), String> {
    match probe {
        vello_common::probe::Probe::Success => Ok(()),
        vello_common::probe::Probe::Error(result) => Err(probe_mismatch_message(&result)),
        vello_common::probe::Probe::RenderError(error) => {
            Err(format!("Probe render failed: {error:?}"))
        }
    }
}

fn probe_mismatch_message(result: &vello_common::probe::ProbeResult) -> String {
    let statistics = result.statistics();
    let failing_features = vello_common::probe::PROBE_ELEMENTS
        .iter()
        .copied()
        .filter(|feature| statistics.differs(*feature))
        .map(|feature| format!("{feature:?}"))
        .collect::<Vec<_>>();

    if failing_features.is_empty() {
        format!(
            "Probe output did not match the bundled reference; failing features could not be isolated (expected {}x{}, actual {}x{})",
            result.expected.width,
            result.expected.height,
            result.actual.width,
            result.actual.height,
        )
    } else {
        format!(
            "Probe output did not match the bundled reference; failing features: {}",
            failing_features.join(", ")
        )
    }
}

struct PendingProbeCompletion {
    started_at: f64,
    start_probe_ms: f64,
    pending_probe: vello_hybrid::WebGlPendingProbe,
}

struct ProbeCompletion {
    readback_ms: f64,
    result: Result<(), String>,
}

struct AppState {
    scenes: Vec<Box<dyn BenchScene>>,
    current_scene: usize,
    backend_caps: BackendCapabilities,
    backend: Box<dyn Backend>,
    canvas: HtmlCanvasElement,
    width: u32,
    height: u32,
    fps_tracker: FpsTracker,
    ui: Ui,
    resources: ResourceStore,
    webgl_init_pending: bool,
    webgl_init_poll_deferred: bool,
    // View state (pan in physical pixels, zoom multiplier).
    pan_x: f64,
    pan_y: f64,
    zoom: f64,
    dragging: bool,
    drag_last_x: f64,
    drag_last_y: f64,
    // Touch state for mobile pan/zoom.
    touch_count: u32,
    touch_last_x: f64,
    touch_last_y: f64,
    /// Distance between two fingers for pinch zoom.
    pinch_dist: f64,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl AppState {
    fn begin_backend_initialization(&mut self, kind: BackendKind) {
        self.webgl_init_pending = kind == BackendKind::Hybrid;
        self.webgl_init_poll_deferred = self.webgl_init_pending;
        if self.webgl_init_pending {
            self.ui.set_webgl_initializing();
        } else {
            self.ui.hide_webgl_init_status();
        }
    }

    fn backend_ready(&mut self) -> bool {
        if self.webgl_init_pending && self.webgl_init_poll_deferred {
            self.webgl_init_poll_deferred = false;
            return false;
        }
        if !self.backend.poll_ready() {
            return false;
        }
        if self.webgl_init_pending {
            self.webgl_init_pending = false;
            self.ui.set_webgl_initialized();
        }
        true
    }

    fn scene_params_for_ui(&self, scene_idx: usize) -> Vec<scenes::Param> {
        scenes::visible_params(self.scenes[scene_idx].as_ref(), self.backend_caps)
    }

    fn switch_backend(&mut self, kind: BackendKind, now: f64) -> bool {
        if self.backend.kind() == kind {
            return false;
        }

        crate::storage::save_backend_name(kind.as_str());
        self.dragging = false;
        self.resources.clear_all(self.backend.as_mut());

        let old_params = self.ui.read_params();

        self.backend_caps = current_backend_capabilities(kind);
        self.canvas = replace_canvas_element(&self.canvas, self.width, self.height);
        self.backend = new_backend(&self.canvas, self.width, self.height, kind);
        self.begin_backend_initialization(kind);
        self.scenes = scenes::all_scenes();

        let next_scene = if self
            .scenes
            .get(self.current_scene)
            .is_some_and(|scene| self.backend_caps.supports_scene(scene.scene_id()))
        {
            self.current_scene
        } else {
            scene_index(scenes::SceneId::Rect)
        };

        self.current_scene = next_scene;
        let scene_id = self.scenes[next_scene].scene_id();
        for (param_id, value) in old_params {
            if self.backend_caps.supports_param(scene_id, param_id)
                && self
                    .backend_caps
                    .supports_param_value(scene_id, param_id, value)
            {
                self.scenes[next_scene].set_param(param_id, value);
            }
        }

        self.ui.set_renderer(kind);
        self.ui
            .rebuild_scene_options(&self.scenes, self.backend_caps, self.current_scene);
        let params = self.scene_params_for_ui(self.current_scene);
        self.ui.rebuild_params(&params);
        let values = self.ui.read_params();
        self.ui.sync_param_visibility(scene_id, &values);
        self.fps_tracker.reset(now);
        self.update_reset_btn();
        self.ui.mark_dirty();
        true
    }

    fn tick(&mut self, now: f64) {
        if !self.backend_ready() {
            self.ui.flush_state();
            return;
        }
        self.tick_interactive(now);
        self.ui.flush_state();
    }

    fn tick_interactive(&mut self, now: f64) {
        let selected = self.ui.selected_scene();
        if selected != self.current_scene && selected < self.scenes.len() {
            let old_scene_id = self.scenes[self.current_scene].scene_id();
            self.resources
                .clear_scene(old_scene_id, self.backend.as_mut());
            self.current_scene = selected;
            let kind = self.backend.kind();
            self.backend = new_backend(&self.canvas, self.width, self.height, kind);
            self.begin_backend_initialization(kind);
            self.scenes = scenes::all_scenes();
            self.fps_tracker.reset(now);
            self.reset_view();
            let params = self.scene_params_for_ui(self.current_scene);
            self.ui.rebuild_params(&params);
            let values = self.ui.read_params();
            self.ui
                .sync_param_visibility(self.scenes[self.current_scene].scene_id(), &values);
            self.ui.mark_dirty();
            return;
        }

        let params = self.ui.read_params();
        let idx = self.current_scene;
        for &(param_id, value) in &params {
            self.scenes[idx].set_param(param_id, value);
        }
        self.ui
            .sync_param_visibility(self.scenes[idx].scene_id(), &params);

        let perf = web_sys::window().unwrap().performance().unwrap();
        let t0 = perf.now();

        self.backend.reset();
        let (w, h) = (self.width, self.height);
        let view = Affine::translate((self.pan_x, self.pan_y)) * Affine::scale(self.zoom);
        self.scenes[idx].render(self.backend.as_mut(), &mut self.resources, w, h, now, view);

        let encode_ms = perf.now() - t0;

        self.backend.render_offscreen();
        let render_ms = perf.now() - t0 - encode_ms;

        self.backend.blit();
        let blit_ms = perf.now() - t0 - encode_ms - render_ms;

        let total_ms = perf.now() - t0;
        let (fps, frame_time) = self.fps_tracker.frame(now);
        let is_cpu = self.backend.is_cpu();
        let supports_encode_timing = self.backend.supports_encode_timing();
        self.ui.update_timing(
            fps,
            frame_time,
            encode_ms,
            render_ms,
            blit_ms,
            total_ms,
            is_cpu,
            supports_encode_timing,
        );
    }

    fn is_view_default(&self) -> bool {
        self.pan_x == 0.0 && self.pan_y == 0.0 && self.zoom == 1.0
    }

    fn update_reset_btn(&self) {
        let display = if self.is_view_default() {
            "none"
        } else {
            "flex"
        };
        self.ui
            .reset_view_btn
            .style()
            .set_property("display", display)
            .unwrap();
    }

    fn reset_view(&mut self) {
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.zoom = 1.0;
        self.update_reset_btn();
    }

    /// Zoom centered on a point in physical pixels.
    fn zoom_at(&mut self, cx: f64, cy: f64, factor: f64) {
        let new_zoom = (self.zoom * factor).clamp(0.05, 100.0);
        let ratio = new_zoom / self.zoom;
        self.pan_x = cx - ratio * (cx - self.pan_x);
        self.pan_y = cy - ratio * (cy - self.pan_y);
        self.zoom = new_zoom;
        self.update_reset_btn();
    }

    fn run_backend_probe(&mut self) -> Option<PendingProbeCompletion> {
        if self.backend.kind() != BackendKind::Hybrid {
            return None;
        }
        let started_at = web_sys::window()
            .and_then(|window| window.performance())
            .map(|performance| performance.now())
            .unwrap_or(0.0);
        self.ui.set_probe_running(true);
        match self.backend.probe() {
            Ok(pending_probe) => {
                let start_probe_ms = probe_elapsed_ms(started_at);
                log::info!("Vello Hybrid probe start_probe finished in {start_probe_ms:.1}ms");
                self.ui.set_probe_sync_complete(start_probe_ms);
                Some(PendingProbeCompletion {
                    started_at,
                    start_probe_ms,
                    pending_probe,
                })
            }
            Err(error) => {
                let start_probe_ms = probe_elapsed_ms(started_at);
                log::warn!(
                    "Vello Hybrid probe failed: start_probe {start_probe_ms:.1}ms, full {start_probe_ms:.1}ms: {error}"
                );
                self.ui
                    .set_probe_failure(&error, start_probe_ms, None, start_probe_ms);
                None
            }
        }
    }

}

fn configure_canvas(canvas: &HtmlCanvasElement, px_w: u32, px_h: u32) {
    canvas.set_width(px_w);
    canvas.set_height(px_h);
    let cs = canvas.style();
    cs.set_property("position", "absolute").unwrap();
    cs.set_property("inset", "0").unwrap();
    cs.set_property("left", "0").unwrap();
    cs.set_property("z-index", "0").unwrap();
    cs.set_property("width", "100%").unwrap();
    cs.set_property("height", "100%").unwrap();
    cs.set_property("display", "block").unwrap();
    cs.set_property("touch-action", "none").unwrap();
}

fn stage_physical_size(document: &web_sys::Document) -> (u32, u32, u32, u32) {
    let stage = document
        .get_element_by_id("canvas-host")
        .expect("canvas-host should exist in index.html");
    let rect = stage.get_bounding_client_rect();
    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    let css_w = rect.width().max(1.0).round() as u32;
    let css_h = rect.height().max(1.0).round() as u32;
    let px_w = (css_w as f64 * dpr).round() as u32;
    let px_h = (css_h as f64 * dpr).round() as u32;
    (css_w, css_h, px_w, px_h)
}

fn make_canvas(document: &web_sys::Document, px_w: u32, px_h: u32) -> HtmlCanvasElement {
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    configure_canvas(&canvas, px_w, px_h);
    canvas
}

fn replace_canvas_element(
    current: &HtmlCanvasElement,
    px_w: u32,
    px_h: u32,
) -> HtmlCanvasElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let new_canvas = make_canvas(&document, px_w, px_h);
    let parent = current.parent_node().unwrap();
    parent.insert_before(&new_canvas, Some(current)).unwrap();
    parent.remove_child(current).unwrap();
    new_canvas
}

fn client_to_canvas_px(canvas: &HtmlCanvasElement, client_x: f64, client_y: f64) -> (f64, f64) {
    let rect = canvas.get_bounding_client_rect();
    let width = rect.width().max(1.0);
    let height = rect.height().max(1.0);
    let x = ((client_x - rect.left()) / width).clamp(0.0, 1.0);
    let y = ((client_y - rect.top()) / height).clamp(0.0, 1.0);
    (x * canvas.width() as f64, y * canvas.height() as f64)
}

fn client_delta_to_canvas_px(canvas: &HtmlCanvasElement, delta_x: f64, delta_y: f64) -> (f64, f64) {
    let rect = canvas.get_bounding_client_rect();
    let scale_x = canvas.width() as f64 / rect.width().max(1.0);
    let scale_y = canvas.height() as f64 / rect.height().max(1.0);
    (delta_x * scale_x, delta_y * scale_y)
}

fn event_target_is_in_stage(target: &wasm_bindgen::JsValue) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Some(document) = window.document() else {
        return false;
    };
    let Some(stage) = document.get_element_by_id("canvas-host") else {
        return false;
    };
    let Ok(node) = target.clone().dyn_into::<web_sys::Node>() else {
        return false;
    };
    stage.contains(Some(&node))
}

/// Entry point.
#[wasm_bindgen]
pub async fn run() {
    init_logging();

    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let performance = window.performance().unwrap();
    let (_, _, px_w, px_h) = stage_physical_size(&document);

    let canvas = make_canvas(&document, px_w, px_h);
    let canvas_host = document
        .get_element_by_id("canvas-host")
        .expect("canvas-host should exist in index.html");
    canvas_host.append_child(&canvas).unwrap();

    let app_scenes = scenes::all_scenes();
    let backend_kind = current_backend_kind();
    let backend_caps = current_backend_capabilities(backend_kind);

    let saved_state = storage::load_ui_state();
    let initial_sidebar_collapsed = saved_state.sidebar_collapsed.unwrap_or(true);
    let initial_scene = saved_state
        .scene
        .filter(|&i| i < app_scenes.len())
        .filter(|&i| backend_caps.supports_scene(app_scenes[i].scene_id()))
        .or_else(|| Some(scene_index(scenes::SceneId::Rect)))
        .unwrap_or(0);

    let ui = Ui::build(
        &document,
        &app_scenes,
        backend_caps,
        initial_scene,
        initial_sidebar_collapsed,
        px_w,
        px_h,
    );
    let mut backend = new_backend(&canvas, px_w, px_h, backend_kind);
    if backend_kind == BackendKind::Hybrid {
        ui.set_webgl_initializing();
        // Allow the browser to present the pending state before polling compilation.
        next_animation_frame().await;
        while !backend.poll_ready() {
            next_animation_frame().await;
        }
        ui.set_webgl_initialized();
    } else {
        ui.hide_webgl_init_status();
    }
    let now = performance.now();

    configure_canvas(&canvas, px_w, px_h);

    let state = Rc::new(RefCell::new(AppState {
        scenes: app_scenes,
        current_scene: initial_scene,
        backend_caps,
        backend,
        canvas,
        width: px_w,
        height: px_h,
        fps_tracker: FpsTracker::new(now),
        ui,
        resources: ResourceStore::new(),
        webgl_init_pending: false,
        webgl_init_poll_deferred: false,
        pan_x: 0.0,
        pan_y: 0.0,
        zoom: 1.0,
        dragging: false,
        drag_last_x: 0.0,
        drag_last_y: 0.0,
        touch_count: 0,
        touch_last_x: 0.0,
        touch_last_y: 0.0,
        pinch_dist: 0.0,
    }));

    {
        let st = state.borrow();
        st.ui.apply_saved_params(&saved_state);
        let values = st.ui.read_params();
        st.ui
            .sync_param_visibility(st.scenes[st.current_scene].scene_id(), &values);
        st.ui.save_state();
    }

    wire_events(&state, &window);
}

/// Wire up all DOM event handlers.
fn wire_events(state: &Rc<RefCell<AppState>>, window: &web_sys::Window) {
    // Sidebar toggle
    {
        let s = state.clone();
        let btn = state.borrow().ui.toggle_btn().clone();
        let cb =
            Closure::wrap(Box::new(move || s.borrow_mut().ui.toggle_sidebar()) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Probe Vello Hybrid from the top bar.
    {
        let btn = state.borrow().ui.top_probe_btn().clone();
        let s = state.clone();
        let cb = Closure::wrap(Box::new(move || {
            if let Some(pending) = s.borrow_mut().run_backend_probe() {
                let s = s.clone();
                poll_probe_completion(pending.pending_probe, move |completion| {
                    let full_ms = probe_elapsed_ms(pending.started_at);
                    let st = s.borrow();
                    match completion.result {
                        Ok(()) => {
                            log::info!(
                                "Vello Hybrid probe succeeded: start_probe {:.1}ms, readback {:.1}ms, full {:.1}ms",
                                pending.start_probe_ms,
                                completion.readback_ms,
                                full_ms
                            );
                            st.ui.set_probe_success(
                                pending.start_probe_ms,
                                completion.readback_ms,
                                full_ms,
                            );
                        }
                        Err(error) => {
                            log::warn!(
                                "Vello Hybrid probe failed: start_probe {:.1}ms, readback {:.1}ms, full {:.1}ms: {error}",
                                pending.start_probe_ms,
                                completion.readback_ms,
                                full_ms
                            );
                            st.ui.set_probe_failure(
                                &error,
                                pending.start_probe_ms,
                                Some(completion.readback_ms),
                                full_ms,
                            );
                        }
                    }
                });
            }
        }) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Reset view button
    {
        let s = state.clone();
        let btn = state.borrow().ui.reset_view_btn.clone();
        let cb = Closure::wrap(Box::new(move || {
            s.borrow_mut().reset_view();
        }) as Box<dyn FnMut()>);
        btn.add_event_listener_with_callback("click", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Scene select → mark dirty
    {
        let dirty = state.borrow().ui.dirty_flag();
        let sel = state.borrow().ui.scene_select.clone();
        let cb = Closure::wrap(Box::new(move || {
            dirty.set(true);
        }) as Box<dyn FnMut()>);
        sel.add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    // Backend select → switch backend at runtime.
    {
        let s = state.clone();
        let select = state.borrow().ui.renderer_select().clone();
        let select_for_cb = select.clone();
        let cb = Closure::wrap(Box::new(move || {
            let mut st = s.borrow_mut();
            let Some(kind) = BackendKind::from_str(&select_for_cb.value()) else {
                st.ui.set_renderer(st.backend.kind());
                return;
            };
            if !kind.is_available() {
                st.ui.set_renderer(st.backend.kind());
                return;
            }
            let now = web_sys::window().unwrap().performance().unwrap().now();
            let replaced_canvas = st.switch_backend(kind, now);
            drop(st);
            if replaced_canvas {
                wire_touch(&s);
            }
        }) as Box<dyn FnMut()>);
        select
            .add_event_listener_with_callback("change", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }

    wire_pan_zoom(state, window);
    wire_touch(state);
    wire_animation_loop(state);
    wire_resize(state);
}

/// Wire pan (mouse drag) and zoom (wheel/pinch) on the window.
fn wire_pan_zoom(state: &Rc<RefCell<AppState>>, window: &web_sys::Window) {
    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
        let mut st = s.borrow_mut();
        if let Some(target) = e.target() {
            if !event_target_is_in_stage(&target) {
                return;
            }
        }
        st.dragging = true;
        st.drag_last_x = e.client_x() as f64;
        st.drag_last_y = e.client_y() as f64;
    }) as Box<dyn FnMut(_)>);
    window
        .add_event_listener_with_callback("mousedown", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();

    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |e: web_sys::MouseEvent| {
        let mut st = s.borrow_mut();
        if !st.dragging {
            return;
        }
        let x = e.client_x() as f64;
        let y = e.client_y() as f64;
        let (dx, dy) =
            client_delta_to_canvas_px(&st.canvas, x - st.drag_last_x, y - st.drag_last_y);
        st.pan_x += dx;
        st.pan_y += dy;
        st.drag_last_x = x;
        st.drag_last_y = y;
        st.update_reset_btn();
    }) as Box<dyn FnMut(_)>);
    window
        .add_event_listener_with_callback("mousemove", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();

    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |_: web_sys::MouseEvent| {
        s.borrow_mut().dragging = false;
    }) as Box<dyn FnMut(_)>);
    window
        .add_event_listener_with_callback("mouseup", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();

    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |e: web_sys::WheelEvent| {
        let mut st = s.borrow_mut();
        e.prevent_default();
        let (cx, cy) = client_to_canvas_px(&st.canvas, e.client_x() as f64, e.client_y() as f64);

        let dy = e.delta_y();
        let scale = if e.ctrl_key() {
            0.01
        } else {
            let line_mult = if e.delta_mode() == 1 { 16.0 } else { 1.0 };
            0.002 * line_mult
        };
        let factor = (-dy * scale).exp();
        st.zoom_at(cx, cy, factor);
    }) as Box<dyn FnMut(_)>);
    let opts = web_sys::AddEventListenerOptions::new();
    opts.set_passive(false);
    window
        .add_event_listener_with_callback_and_add_event_listener_options(
            "wheel",
            cb.as_ref().unchecked_ref(),
            &opts,
        )
        .unwrap();
    cb.forget();
}

/// Helper: compute distance between two touches.
fn touch_distance(t: &web_sys::TouchList) -> f64 {
    if t.length() < 2 {
        return 0.0;
    }
    let a = t.get(0).unwrap();
    let b = t.get(1).unwrap();
    let dx = (a.client_x() - b.client_x()) as f64;
    let dy = (a.client_y() - b.client_y()) as f64;
    (dx * dx + dy * dy).sqrt()
}

/// Helper: compute midpoint of two touches (in client coords).
fn touch_midpoint(t: &web_sys::TouchList) -> (f64, f64) {
    if t.length() < 2 {
        let a = t.get(0).unwrap();
        return (a.client_x() as f64, a.client_y() as f64);
    }
    let a = t.get(0).unwrap();
    let b = t.get(1).unwrap();
    (
        (a.client_x() + b.client_x()) as f64 * 0.5,
        (a.client_y() + b.client_y()) as f64 * 0.5,
    )
}

/// Wire touch events for mobile pan (1 finger) and pinch-to-zoom (2 fingers).
fn wire_touch(state: &Rc<RefCell<AppState>>) {
    let canvas = state.borrow().canvas.clone();
    let target: &web_sys::EventTarget = canvas.as_ref();
    let opts = web_sys::AddEventListenerOptions::new();
    opts.set_passive(false);

    // touchstart
    {
        let s = state.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::TouchEvent| {
            let mut st = s.borrow_mut();
            e.prevent_default();
            let touches = e.touches();
            st.touch_count = touches.length();
            if touches.length() == 1 {
                let t = touches.get(0).unwrap();
                st.touch_last_x = t.client_x() as f64;
                st.touch_last_y = t.client_y() as f64;
            } else if touches.length() >= 2 {
                st.pinch_dist = touch_distance(&touches);
                let (mx, my) = touch_midpoint(&touches);
                st.touch_last_x = mx;
                st.touch_last_y = my;
            }
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback_and_add_event_listener_options(
                "touchstart",
                cb.as_ref().unchecked_ref(),
                &opts,
            )
            .unwrap();
        cb.forget();
    }

    // touchmove
    {
        let s = state.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::TouchEvent| {
            let mut st = s.borrow_mut();
            if st.touch_count == 0 {
                return;
            }
            e.prevent_default();
            let touches = e.touches();

            if touches.length() == 1 && st.touch_count == 1 {
                // Single finger pan.
                let t = touches.get(0).unwrap();
                let x = t.client_x() as f64;
                let y = t.client_y() as f64;
                let (dx, dy) =
                    client_delta_to_canvas_px(&st.canvas, x - st.touch_last_x, y - st.touch_last_y);
                st.pan_x += dx;
                st.pan_y += dy;
                st.touch_last_x = x;
                st.touch_last_y = y;
                st.update_reset_btn();
            } else if touches.length() >= 2 {
                // Pinch zoom + two-finger pan.
                let new_dist = touch_distance(&touches);
                let (mx, my) = touch_midpoint(&touches);

                if st.pinch_dist > 0.0 {
                    let factor = new_dist / st.pinch_dist;
                    let (cx, cy) = client_to_canvas_px(&st.canvas, mx, my);
                    st.zoom_at(cx, cy, factor);
                }
                // Pan by midpoint delta.
                let (dx, dy) = client_delta_to_canvas_px(
                    &st.canvas,
                    mx - st.touch_last_x,
                    my - st.touch_last_y,
                );
                st.pan_x += dx;
                st.pan_y += dy;

                st.pinch_dist = new_dist;
                st.touch_last_x = mx;
                st.touch_last_y = my;
                st.touch_count = touches.length();
            }
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback_and_add_event_listener_options(
                "touchmove",
                cb.as_ref().unchecked_ref(),
                &opts,
            )
            .unwrap();
        cb.forget();
    }

    // touchend / touchcancel
    {
        let s = state.clone();
        let cb = Closure::wrap(Box::new(move |e: web_sys::TouchEvent| {
            let mut st = s.borrow_mut();
            let touches = e.touches();
            st.touch_count = touches.length();
            if touches.length() == 1 {
                // Went from 2→1 finger: reset single-finger tracking.
                let t = touches.get(0).unwrap();
                st.touch_last_x = t.client_x() as f64;
                st.touch_last_y = t.client_y() as f64;
            }
            if touches.length() == 0 {
                st.pinch_dist = 0.0;
            }
        }) as Box<dyn FnMut(_)>);
        target
            .add_event_listener_with_callback("touchend", cb.as_ref().unchecked_ref())
            .unwrap();
        target
            .add_event_listener_with_callback("touchcancel", cb.as_ref().unchecked_ref())
            .unwrap();
        cb.forget();
    }
}

/// Start the requestAnimationFrame loop.
fn wire_animation_loop(state: &Rc<RefCell<AppState>>) {
    let f: RafClosure = Rc::new(RefCell::new(None));
    let g = f.clone();
    let s = state.clone();
    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {
        let now = web_sys::window().unwrap().performance().unwrap().now();
        if let Ok(mut st) = s.try_borrow_mut() {
            st.tick(now);
        }
        request_animation_frame(f.borrow().as_ref().unwrap());
    }) as Box<dyn FnMut()>));
    request_animation_frame(g.borrow().as_ref().unwrap());
}

/// Handle window resize events.
fn wire_resize(state: &Rc<RefCell<AppState>>) {
    let s = state.clone();
    let cb = Closure::wrap(Box::new(move |_: web_sys::Event| {
        let mut st = s.borrow_mut();
        let document = web_sys::window().unwrap().document().unwrap();
        let (_, _, px_w, px_h) = stage_physical_size(&document);
        st.canvas.set_width(px_w);
        st.canvas.set_height(px_h);
        st.width = px_w;
        st.height = px_h;
        st.backend.resize(px_w, px_h);
        st.ui.update_viewport(px_w, px_h);
    }) as Box<dyn FnMut(_)>);
    web_sys::window()
        .unwrap()
        .add_event_listener_with_callback("resize", cb.as_ref().unchecked_ref())
        .unwrap();
    cb.forget();
}

//! Benchmark harness with fixed-size frame averaging.

#![allow(
    clippy::cast_possible_truncation,
    reason = "truncation has no appreciable impact in this benchmark"
)]

use std::collections::{HashMap, HashSet};

use crate::backend::{Backend, current_backend_kind, new_backend};
use crate::resource_store::ResourceStore;
use crate::scenes::{BenchScene, ParamId, SceneId, new_scene};
use crate::storage::CalibrationProfile;
use serde::{Deserialize, Serialize};
use vello_common::kurbo::Affine;
use web_sys::HtmlCanvasElement;

/// A predefined benchmark with fixed parameters.
#[derive(Debug, Clone)]
pub(crate) struct BenchDef {
    /// Display name.
    pub(crate) name: &'static str,
    /// Short description of what this benchmark tests.
    pub(crate) description: &'static str,
    /// Category for grouping in the UI.
    pub(crate) category: &'static str,
    /// Which scene index to use.
    pub(crate) scene_id: SceneId,
    /// Optional count parameter scaled using the shared benchmark scale table.
    pub(crate) scale: Option<BenchScale>,
    /// Parameter overrides (speed is always forced to 0 on top of these).
    pub(crate) params: &'static [(ParamId, f64)],
}

/// Scaling metadata for a benchmark count parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum ScaleGroup {
    Rects5,
    Rects50,
    Rects200,
    Images,
    StrokesLines3,
    StrokesLines20,
    StrokesQuads3,
    StrokesQuads20,
    StrokesCubics3,
    StrokesCubics20,
    FillsPolyline,
    ClipGlobal,
    ClipPerShape,
    Text8,
    Text24,
    Text60,
}

/// Scaling metadata for a benchmark count parameter.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BenchScale {
    pub(crate) param: ParamId,
    pub(crate) group: ScaleGroup,
    pub(crate) default_calibrated_value: usize,
}

/// Result of a single benchmark run.
#[derive(Debug, Clone)]
pub(crate) struct BenchResult {
    /// Benchmark name.
    pub(crate) name: &'static str,
    /// Average time per frame in milliseconds.
    pub(crate) ms_per_frame: f64,
    /// Number of iterations in the run phase.
    pub(crate) iterations: usize,
    /// Total wall-clock time of the run phase in milliseconds.
    #[allow(dead_code, reason = "useful for detailed output")]
    pub(crate) total_ms: f64,
}

/// Events emitted by the harness after each tick.
#[derive(Debug)]
pub(crate) enum HarnessEvent {
    /// A single benchmark finished.
    BenchDone(BenchResult),
    /// All benchmarks finished.
    AllDone,
}

/// Current phase.
#[derive(Debug)]
enum Phase {
    Idle,
    PendingBench(usize),
    Running {
        idx: usize,
        last_now: f64,
        warmup_remaining: usize,
        total_ms: f64,
        samples: usize,
    },
    Complete,
}

/// Orchestrates running benchmarks.
///
/// The harness creates its own fresh context and bench scene instances
/// for each benchmark to ensure complete isolation from interactive mode
/// and between test cases.
pub(crate) struct BenchHarness {
    phase: Phase,
    pub(crate) warmup_samples: usize,
    pub(crate) measured_samples: usize,
    calibration: Option<CalibrationProfile>,
    pub(crate) results: Vec<BenchResult>,
    run_order: Vec<usize>,
    run_pos: usize,
    bench_scene: Option<Box<dyn BenchScene>>,
    bench_canvas: Option<HtmlCanvasElement>,
    bench_backend: Option<Box<dyn Backend>>,
    backend_kind: Option<crate::backend::BackendKind>,
    backend_width: u32,
    backend_height: u32,
    resources: ResourceStore,
}

impl std::fmt::Debug for BenchHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BenchHarness")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl BenchHarness {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::Idle,
            warmup_samples: 3,
            measured_samples: 15,
            calibration: None,
            results: Vec::new(),
            run_order: Vec::new(),
            run_pos: 0,
            bench_scene: None,
            bench_canvas: None,
            bench_backend: None,
            backend_kind: None,
            backend_width: 0,
            backend_height: 0,
            resources: ResourceStore::new(),
        }
    }

    /// Start with a specific set of def indices to run (in order).
    pub(crate) fn start(
        &mut self,
        selected: Vec<usize>,
        _width: u32,
        _height: u32,
        canvas: &HtmlCanvasElement,
    ) {
        self.cleanup_current_bench();
        self.results.clear();
        self.run_order = selected;
        self.run_pos = 0;
        self.bench_canvas = Some(canvas.clone());
        if self.run_order.is_empty() {
            self.phase = Phase::Complete;
        } else {
            self.phase = Phase::PendingBench(self.run_order[0]);
        }
    }

    pub(crate) fn set_calibration(&mut self, calibration: Option<CalibrationProfile>) {
        self.calibration = calibration;
    }

    pub(crate) fn is_running(&self) -> bool {
        !matches!(self.phase, Phase::Idle | Phase::Complete)
    }

    pub(crate) fn current_bench_idx(&self) -> Option<usize> {
        match &self.phase {
            Phase::PendingBench(i) | Phase::Running { idx: i, .. } => Some(*i),
            _ => None,
        }
    }

    /// Drive one step. Returns events for the caller to act on.
    pub(crate) fn tick(
        &mut self,
        defs: &[BenchDef],
        width: u32,
        height: u32,
        now: f64,
    ) -> Vec<HarnessEvent> {
        let mut events = Vec::new();

        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle | Phase::Complete => {}
            Phase::PendingBench(idx) => {
                let def = &defs[idx];
                self.prepare_bench(def, width, height);
                let scene = self.bench_scene.as_mut().unwrap().as_mut();
                let be = self.bench_backend.as_mut().unwrap();
                render_one(scene, be.as_mut(), &mut self.resources, width, height, now);
                self.phase = Phase::Running {
                    idx,
                    last_now: now,
                    warmup_remaining: self.warmup_samples,
                    total_ms: 0.0,
                    samples: 0,
                };
            }
            Phase::Running {
                idx,
                last_now,
                mut warmup_remaining,
                mut total_ms,
                mut samples,
            } => {
                let def = &defs[idx];
                let scene = self.bench_scene.as_mut().unwrap().as_mut();
                let be = self.bench_backend.as_mut().unwrap();
                render_one(scene, be.as_mut(), &mut self.resources, width, height, now);
                let dt = (now - last_now).max(0.0);
                if warmup_remaining > 0 {
                    warmup_remaining -= 1;
                } else {
                    total_ms += dt;
                    samples += 1;
                }

                if samples < self.measured_samples {
                    self.phase = Phase::Running {
                        idx,
                        last_now: now,
                        warmup_remaining,
                        total_ms,
                        samples,
                    };
                } else {
                    let result = BenchResult {
                        name: def.name,
                        ms_per_frame: total_ms / self.measured_samples as f64,
                        iterations: self.measured_samples,
                        total_ms,
                    };
                    self.results.push(result.clone());
                    events.push(HarnessEvent::BenchDone(result));

                    self.run_pos += 1;
                    if self.run_pos < self.run_order.len() {
                        self.phase = Phase::PendingBench(self.run_order[self.run_pos]);
                    } else {
                        self.phase = Phase::Complete;
                        self.cleanup_current_bench();
                        self.bench_canvas = None;
                        events.push(HarnessEvent::AllDone);
                    }
                }
            }
        }

        events
    }

    fn cleanup_current_bench(&mut self) {
        if let Some(backend) = self.bench_backend.as_mut() {
            self.resources.clear_all(backend.as_mut());
        }
        self.bench_scene = None;
    }

    fn prepare_bench(&mut self, def: &BenchDef, width: u32, height: u32) {
        // Note: We reuse the renderer whenever possible. This does have the advantage
        // that some state can leak across benchmarks (for example, if the alpha texture
        // grows very large in one frame then all subsequent benchmarks will also be affected
        // by that). The cleaner way would be to create a new renderer each benchmark,
        // but taht seems to crash my Samsung Tablet very easily (either because of
        // OOM or WebGL context loss), haven't investigated why yet.
        let kind = current_backend_kind();
        let needs_backend_rebuild = self.bench_backend.is_none()
            || self.backend_kind != Some(kind)
            || self.backend_width != width
            || self.backend_height != height;
        if needs_backend_rebuild {
            self.cleanup_current_bench();
            let canvas = self.bench_canvas.as_ref().unwrap();
            self.bench_backend = Some(new_backend(canvas, width, height, kind));
            self.backend_kind = Some(kind);
            self.backend_width = width;
            self.backend_height = height;
        }

        self.bench_scene = Some(new_scene(def.scene_id));
        if let Some(backend) = self.bench_backend.as_mut() {
            self.resources.clear_all(backend.as_mut());
        }
        let scene = self.bench_scene.as_mut().unwrap().as_mut();
        apply_params(scene, def.params, def.scale, self.calibration.as_ref());
    }
}

fn apply_params(
    scene: &mut dyn BenchScene,
    params: &[(ParamId, f64)],
    scale: Option<BenchScale>,
    calibration: Option<&CalibrationProfile>,
) {
    for &(param, value) in params {
        scene.set_param(param, value);
    }
    if let Some(scale) = scale {
        scene.set_param(
            scale.param,
            resolved_or_default_count(scale, calibration) as f64,
        );
    }
    // Always force speed=0 for deterministic benchmarks.
    scene.set_param(ParamId::Speed, 0.0);
}

pub(crate) fn scaled_count(calibrated_value: usize) -> usize {
    const PRESET_16_EXPONENT: f64 = 15.0 / 19.0;
    let max_value = calibrated_value.saturating_mul(4).max(1);
    (max_value as f64).powf(PRESET_16_EXPONENT).ceil().max(1.0) as usize
}

pub(crate) fn default_count(scale: BenchScale) -> usize {
    scaled_count(scale.default_calibrated_value)
}

pub(crate) fn resolved_count(
    scale: BenchScale,
    calibration: Option<&CalibrationProfile>,
) -> Option<usize> {
    calibration.and_then(|profile| profile.count_for(scale.group))
}

pub(crate) fn resolved_or_default_count(
    scale: BenchScale,
    calibration: Option<&CalibrationProfile>,
) -> usize {
    resolved_count(scale, calibration).unwrap_or_else(|| default_count(scale))
}

fn render_one(
    bench_scene: &mut dyn BenchScene,
    backend: &mut dyn Backend,
    resources: &mut ResourceStore,
    width: u32,
    height: u32,
    time: f64,
) {
    backend.reset();
    bench_scene.render(backend, resources, width, height, time, Affine::IDENTITY);
    backend.render_offscreen();
    backend.blit();
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CalibrationTarget {
    pub(crate) group: ScaleGroup,
    pub(crate) label: &'static str,
    pub(crate) scene_id: SceneId,
    pub(crate) scale_param: ParamId,
    pub(crate) params: &'static [(ParamId, f64)],
}

#[derive(Debug, Clone)]
pub(crate) enum CalibrationEvent {
    AllDone(HashMap<ScaleGroup, usize>),
}

#[derive(Debug, Clone, Copy)]
struct ProbeResult {
    count: usize,
    ms: f64,
}

#[derive(Debug)]
enum CalibrationPhase {
    Idle,
    PendingProbe {
        target_idx: usize,
        probe_count: usize,
        lower: Option<ProbeResult>,
        upper: Option<ProbeResult>,
        best: Option<ProbeResult>,
        binary_steps_left: u8,
    },
    Running {
        target_idx: usize,
        probe_count: usize,
        lower: Option<ProbeResult>,
        upper: Option<ProbeResult>,
        best: Option<ProbeResult>,
        binary_steps_left: u8,
        last_now: f64,
        warmup_remaining: usize,
        total_ms: f64,
        samples: usize,
    },
    Complete,
}

pub(crate) struct CalibrationHarness {
    phase: CalibrationPhase,
    targets: Vec<CalibrationTarget>,
    results: HashMap<ScaleGroup, usize>,
    target_ms: f64,
    active_target_idx: Option<usize>,
    bench_scene: Option<Box<dyn BenchScene>>,
    bench_canvas: Option<HtmlCanvasElement>,
    bench_backend: Option<Box<dyn Backend>>,
    backend_kind: Option<crate::backend::BackendKind>,
    backend_width: u32,
    backend_height: u32,
    resources: ResourceStore,
}

impl std::fmt::Debug for CalibrationHarness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CalibrationHarness")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl CalibrationHarness {
    pub(crate) fn new() -> Self {
        Self {
            phase: CalibrationPhase::Idle,
            targets: Vec::new(),
            results: HashMap::new(),
            target_ms: 80.0,
            active_target_idx: None,
            bench_scene: None,
            bench_canvas: None,
            bench_backend: None,
            backend_kind: None,
            backend_width: 0,
            backend_height: 0,
            resources: ResourceStore::new(),
        }
    }

    pub(crate) fn start(
        &mut self,
        defs: &[BenchDef],
        canvas: &HtmlCanvasElement,
        width: u32,
        height: u32,
    ) {
        self.cleanup_current_probe();
        self.targets = calibration_targets(defs);
        self.results.clear();
        self.active_target_idx = None;
        self.bench_canvas = Some(canvas.clone());
        if self.targets.is_empty() {
            self.phase = CalibrationPhase::Complete;
        } else {
            self.phase = CalibrationPhase::PendingProbe {
                target_idx: 0,
                probe_count: 1,
                lower: None,
                upper: None,
                best: None,
                binary_steps_left: 7,
            };
        }
        self.backend_width = width;
        self.backend_height = height;
    }

    pub(crate) fn is_running(&self) -> bool {
        !matches!(
            self.phase,
            CalibrationPhase::Idle | CalibrationPhase::Complete
        )
    }

    pub(crate) fn current_status(&self) -> Option<String> {
        let (target_idx, probe_count) = match &self.phase {
            CalibrationPhase::PendingProbe {
                target_idx,
                probe_count,
                ..
            }
            | CalibrationPhase::Running {
                target_idx,
                probe_count,
                ..
            } => (*target_idx, *probe_count),
            _ => return None,
        };
        let target = self.targets.get(target_idx)?;
        Some(format!(
            "Calibrating {}/{}: {} ({})",
            target_idx + 1,
            self.targets.len(),
            target.label,
            probe_count,
        ))
    }

    pub(crate) fn tick(&mut self, width: u32, height: u32, now: f64) -> Vec<CalibrationEvent> {
        let mut events = Vec::new();
        match std::mem::replace(&mut self.phase, CalibrationPhase::Idle) {
            CalibrationPhase::Idle | CalibrationPhase::Complete => {}
            CalibrationPhase::PendingProbe {
                target_idx,
                probe_count,
                lower,
                upper,
                best,
                binary_steps_left,
            } => {
                self.prepare_probe(target_idx, probe_count, width, height);
                let target = self.targets[target_idx];
                let scene = self.bench_scene.as_mut().unwrap().as_mut();
                let backend = self.bench_backend.as_mut().unwrap();
                render_calibration_probe(
                    scene,
                    backend.as_mut(),
                    &mut self.resources,
                    width,
                    height,
                    now,
                    target.scale_param,
                    probe_count,
                );
                self.phase = CalibrationPhase::Running {
                    target_idx,
                    probe_count,
                    lower,
                    upper,
                    best,
                    binary_steps_left,
                    last_now: now,
                    warmup_remaining: 3,
                    total_ms: 0.0,
                    samples: 0,
                };
            }
            CalibrationPhase::Running {
                target_idx,
                probe_count,
                lower,
                upper,
                best,
                binary_steps_left,
                last_now,
                mut warmup_remaining,
                mut total_ms,
                mut samples,
            } => {
                let target = self.targets[target_idx];
                let scene = self.bench_scene.as_mut().unwrap().as_mut();
                let backend = self.bench_backend.as_mut().unwrap();
                render_calibration_probe(
                    scene,
                    backend.as_mut(),
                    &mut self.resources,
                    width,
                    height,
                    now,
                    target.scale_param,
                    probe_count,
                );
                let dt = (now - last_now).max(0.0);
                if warmup_remaining > 0 {
                    warmup_remaining -= 1;
                } else {
                    total_ms += dt;
                    samples += 1;
                }

                if samples < 5 {
                    self.phase = CalibrationPhase::Running {
                        target_idx,
                        probe_count,
                        lower,
                        upper,
                        best,
                        binary_steps_left,
                        last_now: now,
                        warmup_remaining,
                        total_ms,
                        samples,
                    };
                } else {
                    let measured = ProbeResult {
                        count: probe_count,
                        ms: total_ms / 5.0,
                    };
                    match self.advance_calibration(
                        target_idx,
                        lower,
                        upper,
                        best,
                        binary_steps_left,
                        measured,
                    ) {
                        Ok(next_phase) => {
                            self.phase = next_phase;
                        }
                        Err(best_count) => {
                            self.results.insert(target.group, best_count);
                            if target_idx + 1 < self.targets.len() {
                                self.phase = CalibrationPhase::PendingProbe {
                                    target_idx: target_idx + 1,
                                    probe_count: 1,
                                    lower: None,
                                    upper: None,
                                    best: None,
                                    binary_steps_left: 7,
                                };
                            } else {
                                self.phase = CalibrationPhase::Complete;
                                self.cleanup_current_probe();
                                self.bench_canvas = None;
                                events.push(CalibrationEvent::AllDone(self.results.clone()));
                            }
                        }
                    }
                }
            }
        }
        events
    }

    fn cleanup_current_probe(&mut self) {
        if let Some(backend) = self.bench_backend.as_mut() {
            self.resources.clear_all(backend.as_mut());
        }
        self.bench_scene = None;
        self.active_target_idx = None;
    }

    fn prepare_probe(&mut self, target_idx: usize, probe_count: usize, width: u32, height: u32) {
        let kind = current_backend_kind();
        let needs_backend_rebuild = self.bench_backend.is_none()
            || self.backend_kind != Some(kind)
            || self.backend_width != width
            || self.backend_height != height;
        if needs_backend_rebuild {
            self.cleanup_current_probe();
            let canvas = self.bench_canvas.as_ref().unwrap();
            self.bench_backend = Some(new_backend(canvas, width, height, kind));
            self.backend_kind = Some(kind);
            self.backend_width = width;
            self.backend_height = height;
        }

        if self.active_target_idx != Some(target_idx) || self.bench_scene.is_none() {
            let target = self.targets[target_idx];
            if let Some(backend) = self.bench_backend.as_mut() {
                self.resources.clear_all(backend.as_mut());
            }
            self.bench_scene = Some(new_scene(target.scene_id));
            let scene = self.bench_scene.as_mut().unwrap().as_mut();
            apply_params(scene, target.params, None, None);
            self.active_target_idx = Some(target_idx);
        }
        let target = self.targets[target_idx];
        self.bench_scene
            .as_mut()
            .unwrap()
            .as_mut()
            .set_param(target.scale_param, probe_count as f64);
    }

    fn advance_calibration(
        &self,
        target_idx: usize,
        lower: Option<ProbeResult>,
        upper: Option<ProbeResult>,
        best: Option<ProbeResult>,
        binary_steps_left: u8,
        measured: ProbeResult,
    ) -> Result<CalibrationPhase, usize> {
        let best = choose_better(best, measured, self.target_ms);
        if within_target_tolerance(measured.ms, self.target_ms) {
            return Err(measured.count);
        }
        match upper {
            None if measured.ms < self.target_ms => {
                if measured.count >= 10_000_000 {
                    return Err(best.count);
                }
                Ok(CalibrationPhase::PendingProbe {
                    target_idx,
                    probe_count: next_probe_count(measured.count),
                    lower: Some(measured),
                    upper: None,
                    best: Some(best),
                    binary_steps_left,
                })
            }
            None => {
                if let Some(lower) = lower {
                    let next = midpoint(lower.count, measured.count);
                    if next == lower.count || next == measured.count {
                        Err(best.count)
                    } else {
                        Ok(CalibrationPhase::PendingProbe {
                            target_idx,
                            probe_count: next,
                            lower: Some(lower),
                            upper: Some(measured),
                            best: Some(best),
                            binary_steps_left,
                        })
                    }
                } else {
                    Err(best.count)
                }
            }
            Some(upper) => {
                let (lower, upper) = if measured.ms < self.target_ms {
                    (Some(measured), upper)
                } else {
                    (lower, measured)
                };
                let Some(lower) = lower else {
                    return Err(best.count);
                };
                if upper.count <= lower.count + 1 || binary_steps_left == 0 {
                    Err(best.count)
                } else {
                    let next = midpoint(lower.count, upper.count);
                    if next == lower.count || next == upper.count {
                        Err(best.count)
                    } else {
                        Ok(CalibrationPhase::PendingProbe {
                            target_idx,
                            probe_count: next,
                            lower: Some(lower),
                            upper: Some(upper),
                            best: Some(best),
                            binary_steps_left: binary_steps_left - 1,
                        })
                    }
                }
            }
        }
    }
}

fn choose_better(
    current: Option<ProbeResult>,
    candidate: ProbeResult,
    target_ms: f64,
) -> ProbeResult {
    match current {
        Some(current) if (current.ms - target_ms).abs() < (candidate.ms - target_ms).abs() => {
            current
        }
        Some(current)
            if (current.ms - target_ms).abs() == (candidate.ms - target_ms).abs()
                && current.count < candidate.count =>
        {
            current
        }
        _ => candidate,
    }
}

fn within_target_tolerance(ms: f64, target_ms: f64) -> bool {
    (ms - target_ms).abs() <= 5.0
}

fn midpoint(low: usize, high: usize) -> usize {
    low + (high - low) / 2
}

fn next_probe_count(count: usize) -> usize {
    if count < 10 {
        count + 1
    } else {
        let step = 10usize.pow(count.ilog10());
        count + step
    }
}

fn render_calibration_probe(
    bench_scene: &mut dyn BenchScene,
    backend: &mut dyn Backend,
    resources: &mut ResourceStore,
    width: u32,
    height: u32,
    time: f64,
    scale_param: ParamId,
    probe_count: usize,
) {
    bench_scene.set_param(scale_param, probe_count as f64);
    render_one(bench_scene, backend, resources, width, height, time);
}

fn is_calibration_representative(def: &BenchDef) -> bool {
    matches!(
        def.category,
        "Rects (alpha)"
            | "Images (alpha)"
            | "Strokes (alpha)"
            | "Fills"
            | "Clip Paths (alpha)"
            | "Text (alpha)"
    )
}

pub(crate) fn calibration_targets(defs: &[BenchDef]) -> Vec<CalibrationTarget> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for def in defs {
        let Some(scale) = def.scale else {
            continue;
        };
        if !is_calibration_representative(def) || !seen.insert(scale.group) {
            continue;
        }
        out.push(CalibrationTarget {
            group: scale.group,
            label: def.name,
            scene_id: def.scene_id,
            scale_param: scale.param,
            params: def.params,
        });
    }
    out
}

/// All predefined benchmarks.
pub(crate) fn bench_defs() -> Vec<BenchDef> {
    vec![
        // ── Rects (alpha) ──────────────────────────────────────────────
        BenchDef {
            name: "Rect - 5×5 - Solid",
            description: "rendering small semi-transparent rectangles",
            category: "Rects (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Rects5,
                default_calibrated_value: 3_044_740,
            }),
            params: &[
                (ParamId::NumRects, 3_044_740.0),
                (ParamId::RectSize, 5.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 50×50 - Solid",
            description: "rendering medium semi-transparent rectangles",
            category: "Rects (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Rects50,
                default_calibrated_value: 1_012_253,
            }),
            params: &[
                (ParamId::NumRects, 1_012_253.0),
                (ParamId::RectSize, 50.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Solid",
            description: "rendering large semi-transparent rectangles",
            category: "Rects (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Rects200,
                default_calibrated_value: 68_803,
            }),
            params: &[
                (ParamId::NumRects, 68_803.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        // ── Rects (alpha, low overdraw) ──────────────────────────────
        // TargetOverlap keeps the average per-pixel overlap ratio constant
        // as NumRects scales with preset: rect size shrinks to compensate.
        // Low overlap means dest.a never fully saturates. This has been found to
        // have a large impact on pipeline architecture.
        BenchDef {
            name: "Rect - 2x Overlap",
            description: "alpha rects, ~2x avg per-pixel overlap — rect size adapts to viewport",
            category: "Rects (alpha, low overdraw)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Rects50,
                default_calibrated_value: 1_012_253,
            }),
            params: &[
                (ParamId::NumRects, 1_012_253.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
                (ParamId::TargetOverlap, 2.0),
            ],
        },
        BenchDef {
            name: "Rect - 4x Overlap",
            description: "alpha rects, ~4x avg per-pixel overlap — rect size adapts to viewport",
            category: "Rects (alpha, low overdraw)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Rects50,
                default_calibrated_value: 1_012_253,
            }),
            params: &[
                (ParamId::NumRects, 1_012_253.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 0.0),
                (ParamId::TargetOverlap, 4.0),
            ],
        },
        // ── Rects (opaque) ─────────────────────────────────────────────
        BenchDef {
            name: "Rect - 5×5 - Solid (opaque)",
            description: "rendering small fully opaque rectangles",
            category: "Rects (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Rects5,
                default_calibrated_value: 3_044_740,
            }),
            params: &[
                (ParamId::NumRects, 3_044_740.0),
                (ParamId::RectSize, 5.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 50×50 - Solid (opaque)",
            description: "rendering medium fully opaque rectangles",
            category: "Rects (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Rects50,
                default_calibrated_value: 1_012_253,
            }),
            params: &[
                (ParamId::NumRects, 1_012_253.0),
                (ParamId::RectSize, 50.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Solid (opaque)",
            description: "rendering large fully opaque rectangles",
            category: "Rects (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Rects200,
                default_calibrated_value: 68_803,
            }),
            params: &[
                (ParamId::NumRects, 68_803.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 0.0),
                (ParamId::Rotated, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        // ── Images (alpha) ─────────────────────────────────────────────
        BenchDef {
            name: "Rect - 200×200 - Image - Nearest",
            description: "rendering transparent images with NN sampling",
            category: "Images (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Images,
                default_calibrated_value: 25_923,
            }),
            params: &[
                (ParamId::NumRects, 25_923.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 0.0),
                (ParamId::ImageOpaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Image - Bilinear",
            description: "rendering transparent images with bilinear sampling",
            category: "Images (alpha)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Images,
                default_calibrated_value: 25_923,
            }),
            params: &[
                (ParamId::NumRects, 20_553.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 1.0),
                (ParamId::ImageOpaque, 0.0),
            ],
        },
        // ── Images (alpha, low overdraw) ──────────────────────────────
        // Image rects with alpha go entirely through the alpha pass (atlas
        // textures have transparency).
        BenchDef {
            name: "Image - 2x Overlap - Nearest",
            description: "alpha images, ~2x avg overlap, NN sampling — rect size adapts to viewport",
            category: "Images (alpha, low overdraw)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Images,
                default_calibrated_value: 25_923,
            }),
            params: &[
                (ParamId::NumRects, 25_923.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 0.0),
                (ParamId::ImageOpaque, 0.0),
                (ParamId::TargetOverlap, 2.0),
            ],
        },
        BenchDef {
            name: "Image - 4x Overlap - Nearest",
            description: "alpha images, ~4x avg overlap, NN sampling — rect size adapts to viewport",
            category: "Images (alpha, low overdraw)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Images,
                default_calibrated_value: 25_923,
            }),
            params: &[
                (ParamId::NumRects, 25_923.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 0.0),
                (ParamId::ImageOpaque, 0.0),
                (ParamId::TargetOverlap, 4.0),
            ],
        },
        // ── Images (opaque) ────────────────────────────────────────────
        BenchDef {
            name: "Rect - 200×200 - Opaque Image - Nearest",
            description: "rendering opaque images with NN sampling",
            category: "Images (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Images,
                default_calibrated_value: 25_923,
            }),
            params: &[
                (ParamId::NumRects, 25_923.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 0.0),
                (ParamId::ImageOpaque, 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200×200 - Opaque Image - Bilinear",
            description: "rendering opaque images with bilinear sampling",
            category: "Images (opaque)",
            scene_id: SceneId::Rect,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::Images,
                default_calibrated_value: 25_923,
            }),
            params: &[
                (ParamId::NumRects, 20_553.0),
                (ParamId::RectSize, 200.0),
                (ParamId::PaintMode, 2.0),
                (ParamId::Rotated, 0.0),
                (ParamId::ImageFilter, 1.0),
                (ParamId::ImageOpaque, 1.0),
            ],
        },
        // BenchDef {
        //     name: "Rect - 200×200 - Opaque Image (draw_image) - Nearest",
        //     description: "rendering images via draw_image API (GPU fast path on hybrid)",
        //     category: "Images (opaque)",
        //     scene_id: SceneId::Rect,
        //     scale: Some(BenchScale {
        //         param: ParamId::NumRects,
        //         calibrated_value: 35_000,
        //     }),
        //     params: &[
        //         (ParamId::NumRects, 35_000.0),
        //         (ParamId::RectSize, 200.0),
        //         (ParamId::PaintMode, 2.0),
        //         (ParamId::Rotated, 0.0),
        //         (ParamId::ImageFilter, 0.0),
        //         (ParamId::ImageOpaque, 1.0),
        //         (ParamId::UseDrawImage, 1.0),
        //     ],
        // },
        // BenchDef {
        //     name: "Rect - 200×200 - Opaque Image (draw_image) - Bilinear",
        //     description: "rendering images via draw_image API with bilinear (GPU fast path on hybrid)",
        //     category: "Images (opaque)",
        //     scene_id: SceneId::Rect,
        //     scale: Some(BenchScale {
        //         param: ParamId::NumRects,
        //         calibrated_value: 34_000,
        //     }),
        //     params: &[
        //         (ParamId::NumRects, 34_000.0),
        //         (ParamId::RectSize, 200.0),
        //         (ParamId::PaintMode, 2.0),
        //         (ParamId::Rotated, 0.0),
        //         (ParamId::ImageFilter, 1.0),
        //         (ParamId::ImageOpaque, 1.0),
        //         (ParamId::UseDrawImage, 1.0),
        //     ],
        // },
        // ── Strokes (alpha) ────────────────────────────────────────────
        BenchDef {
            name: "Stroked Lines - 3px",
            description: "rendering semi-transparent lines with small stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesLines3,
                default_calibrated_value: 75_995,
            }),
            params: &[
                (ParamId::NumStrokes, 75_995.0),
                (ParamId::CurveType, 0.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Lines - 20px",
            description: "rendering semi-transparent lines with large stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesLines20,
                default_calibrated_value: 48_065,
            }),
            params: &[
                (ParamId::NumStrokes, 48_065.0),
                (ParamId::CurveType, 0.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Quads - 3px",
            description: "rendering semi-transparent quads with small stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesQuads3,
                default_calibrated_value: 22_614,
            }),
            params: &[
                (ParamId::NumStrokes, 22_614.0),
                (ParamId::CurveType, 1.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Quads - 20px",
            description: "rendering semi-transparent quads with large stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesQuads20,
                default_calibrated_value: 14_529,
            }),
            params: &[
                (ParamId::NumStrokes, 14_529.0),
                (ParamId::CurveType, 1.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 3px",
            description: "rendering semi-transparent cubics with small stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesCubics3,
                default_calibrated_value: 15_987,
            }),
            params: &[
                (ParamId::NumStrokes, 15_987.0),
                (ParamId::CurveType, 2.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 20px",
            description: "rendering semi-transparent cubics with large stroke width",
            category: "Strokes (alpha)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesCubics20,
                default_calibrated_value: 9_428,
            }),
            params: &[
                (ParamId::NumStrokes, 9_428.0),
                (ParamId::CurveType, 2.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        // ── Strokes (opaque) ───────────────────────────────────────────
        BenchDef {
            name: "Stroked Lines - 3px (opaque)",
            description: "rendering opaque lines with small stroke width",
            category: "Strokes (opaque)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesLines3,
                default_calibrated_value: 75_995,
            }),
            params: &[
                (ParamId::NumStrokes, 75_995.0),
                (ParamId::CurveType, 0.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Stroked Lines - 20px (opaque)",
            description: "rendering opaque lines with large stroke width",
            category: "Strokes (opaque)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesLines20,
                default_calibrated_value: 48_065,
            }),
            params: &[
                (ParamId::NumStrokes, 48_065.0),
                (ParamId::CurveType, 0.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 3px (opaque)",
            description: "rendering opaque cubics with small stroke width",
            category: "Strokes (opaque)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesCubics3,
                default_calibrated_value: 15_987,
            }),
            params: &[
                (ParamId::NumStrokes, 15_987.0),
                (ParamId::CurveType, 2.0),
                (ParamId::StrokeWidth, 3.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Stroked Cubics - 20px (opaque)",
            description: "rendering opaque cubics with large stroke width",
            category: "Strokes (opaque)",
            scene_id: SceneId::Strokes,
            scale: Some(BenchScale {
                param: ParamId::NumStrokes,
                group: ScaleGroup::StrokesCubics20,
                default_calibrated_value: 9_428,
            }),
            params: &[
                (ParamId::NumStrokes, 9_428.0),
                (ParamId::CurveType, 2.0),
                (ParamId::StrokeWidth, 20.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Polyline",
            description: "rendering paths bottlenecked by tiling and strip rendering",
            category: "Fills",
            scene_id: SceneId::Polyline,
            scale: Some(BenchScale {
                param: ParamId::NumVertices,
                group: ScaleGroup::FillsPolyline,
                default_calibrated_value: 6_462,
            }),
            params: &[(ParamId::NumVertices, 6462.0)],
        },
        BenchDef {
            name: "Ghostscript Tiger",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_id: SceneId::Svg,
            scale: None,
            params: &[(ParamId::SvgAsset, 0.0)],
        },
        BenchDef {
            name: "Coat of Arms",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_id: SceneId::Svg,
            scale: None,
            params: &[(ParamId::SvgAsset, 1.0)],
        },
        BenchDef {
            name: "Heraldry",
            description: "rendering simple vector graphics",
            category: "Vector Graphics",
            scene_id: SceneId::Svg,
            scale: None,
            params: &[(ParamId::SvgAsset, 2.0)],
        },
        // ── Clip Paths (alpha) ─────────────────────────────────────────
        BenchDef {
            name: "Rect - 400px - Global `clip_path`",
            description: "rendering many semi-transparent paths with a single clip path",
            category: "Clip Paths (alpha)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::ClipGlobal,
                default_calibrated_value: 4_890,
            }),
            params: &[
                (ParamId::NumRects, 4_890.0),
                (ParamId::RectSize, 400.0),
                (ParamId::ClipMode, 1.0),
                (ParamId::ClipMethod, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 400px - Global `clip_layer`",
            description: "rendering many semi-transparent paths with a single clip layer",
            category: "Clip Paths (alpha)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::ClipGlobal,
                default_calibrated_value: 4_890,
            }),
            params: &[
                (ParamId::NumRects, 4_890.0),
                (ParamId::RectSize, 400.0),
                (ParamId::ClipMode, 1.0),
                (ParamId::ClipMethod, 1.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200px - Per-shape `clip_path`",
            description: "rendering many semi-transparent paths with many clip paths",
            category: "Clip Paths (alpha)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::ClipPerShape,
                default_calibrated_value: 1_867,
            }),
            params: &[
                (ParamId::NumRects, 1_867.0),
                (ParamId::RectSize, 200.0),
                (ParamId::ClipMode, 2.0),
                (ParamId::ClipMethod, 0.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Rect - 200px - Per-shape `clip_layer`",
            description: "rendering many semi-transparent paths with many clip layers",
            category: "Clip Paths (alpha)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::ClipPerShape,
                default_calibrated_value: 1_867,
            }),
            params: &[
                (ParamId::NumRects, 1_867.0),
                (ParamId::RectSize, 200.0),
                (ParamId::ClipMode, 2.0),
                (ParamId::ClipMethod, 1.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        // ── Clip Paths (opaque) ────────────────────────────────────────
        BenchDef {
            name: "Rect - 400px - Global `clip_path` (opaque)",
            description: "rendering many opaque paths with a single clip path",
            category: "Clip Paths (opaque)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::ClipGlobal,
                default_calibrated_value: 4_890,
            }),
            params: &[
                (ParamId::NumRects, 4_890.0),
                (ParamId::RectSize, 400.0),
                (ParamId::ClipMode, 1.0),
                (ParamId::ClipMethod, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Rect - 200px - Per-shape `clip_path` (opaque)",
            description: "rendering many opaque paths with many clip paths",
            category: "Clip Paths (opaque)",
            scene_id: SceneId::Clip,
            scale: Some(BenchScale {
                param: ParamId::NumRects,
                group: ScaleGroup::ClipPerShape,
                default_calibrated_value: 1_867,
            }),
            params: &[
                (ParamId::NumRects, 1_867.0),
                (ParamId::RectSize, 200.0),
                (ParamId::ClipMode, 2.0),
                (ParamId::ClipMethod, 0.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        // ── Text (alpha) ───────────────────────────────────────────────
        BenchDef {
            name: "Text - 8px",
            description: "rendering small semi-transparent text",
            category: "Text (alpha)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                group: ScaleGroup::Text8,
                default_calibrated_value: 6_460,
            }),
            params: &[
                (ParamId::NumRuns, 6_460.0),
                (ParamId::FontSize, 8.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Text - 24px",
            description: "rendering medium-sized semi-transparent text",
            category: "Text (alpha)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                group: ScaleGroup::Text24,
                default_calibrated_value: 4_536,
            }),
            params: &[
                (ParamId::NumRuns, 4_536.0),
                (ParamId::FontSize, 24.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        BenchDef {
            name: "Text - 60px",
            description: "rendering large semi-transparent text",
            category: "Text (alpha)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                group: ScaleGroup::Text60,
                default_calibrated_value: 2_273,
            }),
            params: &[
                (ParamId::NumRuns, 2_273.0),
                (ParamId::FontSize, 60.0),
                (ParamId::Opaque, 0.0),
            ],
        },
        // ── Text (opaque) ──────────────────────────────────────────────
        BenchDef {
            name: "Text - 8px (opaque)",
            description: "rendering small opaque text",
            category: "Text (opaque)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                group: ScaleGroup::Text8,
                default_calibrated_value: 6_460,
            }),
            params: &[
                (ParamId::NumRuns, 6_460.0),
                (ParamId::FontSize, 8.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Text - 24px (opaque)",
            description: "rendering medium-sized opaque text",
            category: "Text (opaque)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                group: ScaleGroup::Text24,
                default_calibrated_value: 4_536,
            }),
            params: &[
                (ParamId::NumRuns, 4_536.0),
                (ParamId::FontSize, 24.0),
                (ParamId::Opaque, 1.0),
            ],
        },
        BenchDef {
            name: "Text - 60px (opaque)",
            description: "rendering large opaque text",
            category: "Text (opaque)",
            scene_id: SceneId::Text,
            scale: Some(BenchScale {
                param: ParamId::NumRuns,
                group: ScaleGroup::Text60,
                default_calibrated_value: 2_273,
            }),
            params: &[
                (ParamId::NumRuns, 2_273.0),
                (ParamId::FontSize, 60.0),
                (ParamId::Opaque, 1.0),
            ],
        },
    ]
}

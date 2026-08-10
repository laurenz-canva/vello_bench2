use crate::renderer::{BenchRenderer, RenderMode};

#[derive(Clone, Debug)]
pub struct BenchConfig {
    pub image_counts: Vec<usize>,
    pub texture_size: u16,
    pub draw_size: u16,
    pub warmup_seconds: f64,
    pub measurement_seconds: f64,
}

impl BenchConfig {
    pub fn validate(&mut self, max_texture_size: u32) -> Result<(), String> {
        self.image_counts.sort_unstable();
        self.image_counts.dedup();

        if self.image_counts.is_empty() {
            return Err("provide at least one image count".to_string());
        }
        if self.image_counts[0] == 0 {
            return Err("image counts must be greater than zero".to_string());
        }
        if self.texture_size < 4 {
            return Err("the texture size must be at least 4 pixels".to_string());
        }
        if u32::from(self.texture_size) > max_texture_size {
            return Err(format!(
                "the requested texture exceeds this device's {max_texture_size}px limit"
            ));
        }
        if !self.warmup_seconds.is_finite()
            || self.warmup_seconds <= 0.0
            || !self.measurement_seconds.is_finite()
            || self.measurement_seconds <= 0.0
        {
            return Err("use positive warmup and measurement durations".to_string());
        }
        Ok(())
    }

    pub fn variant_count(&self) -> usize {
        self.image_counts.len()
    }
}

#[derive(Clone, Debug)]
pub struct MeasurementMetrics {
    pub fps: f64,
    pub cpu_ms: f64,
}

#[derive(Clone, Debug)]
pub struct CaseResult {
    pub image_count: usize,
    pub image_fps: f64,
    pub external_fps: f64,
    pub delta_fps_percent: f64,
    pub image_cpu_ms: f64,
    pub external_cpu_ms: f64,
    pub delta_cpu_percent: f64,
}

#[derive(Debug)]
pub enum RunnerEvent {
    Status(String),
    Progress { completed: usize, total: usize },
    UploadProgress { uploaded: usize, total: usize },
    UploadComplete,
    CaseComplete(CaseResult),
    Complete(String),
    Failed(String),
}

#[derive(Debug)]
enum Phase {
    Idle,
    Preparing {
        target_count: usize,
    },
    FinishingUploads,
    Warming {
        mode: RenderMode,
        elapsed_ms: f64,
        last_raf: Option<f64>,
    },
    Measuring {
        mode: RenderMode,
        frame_intervals: Vec<f64>,
        cpu_times: Vec<f64>,
        last_raf: Option<f64>,
    },
    Complete,
}

pub struct BenchRunner {
    config: Option<BenchConfig>,
    phase: Phase,
    mode: RenderMode,
    count_index: usize,
    case_limit: usize,
    image_results: Vec<MeasurementMetrics>,
    run_generation: u64,
    image_seed: u64,
}

impl BenchRunner {
    pub fn new() -> Self {
        Self {
            config: None,
            phase: Phase::Idle,
            mode: RenderMode::ImagePaint,
            count_index: 0,
            case_limit: 0,
            image_results: Vec::new(),
            run_generation: 0,
            image_seed: 0,
        }
    }

    pub fn start(&mut self, config: BenchConfig, renderer: &mut BenchRenderer) -> Vec<RunnerEvent> {
        let size = config.texture_size;
        self.config = Some(config);
        self.mode = RenderMode::ImagePaint;
        self.count_index = 0;
        self.case_limit = self.config.as_ref().unwrap().variant_count();
        self.image_results.clear();
        self.run_generation = self.run_generation.wrapping_add(1);
        self.image_seed = self.run_generation.wrapping_mul(0xD1B5_4A32_D192_ED03);
        renderer.begin_benchmark_texture_set(size, self.mode, self.image_seed);
        self.prepare_current_case(renderer)
    }

    pub fn stop(&mut self) {
        self.phase = Phase::Idle;
        self.config = None;
    }

    pub fn invalidate_pending_timing(&mut self) {
        match &mut self.phase {
            Phase::Warming { last_raf, .. } => *last_raf = None,
            Phase::Measuring {
                frame_intervals,
                cpu_times,
                last_raf,
                ..
            } => {
                frame_intervals.clear();
                cpu_times.clear();
                *last_raf = None;
            }
            _ => {}
        }
    }

    /// Advance exactly one browser animation frame and submit exactly one Vello render.
    pub fn tick(&mut self, renderer: &mut BenchRenderer, now: f64) -> Vec<RunnerEvent> {
        let mut events = Vec::new();
        let Some(config) = self.config.as_ref() else {
            return events;
        };
        let warmup_ms = config.warmup_seconds * 1000.0;
        let measurement_ms = config.measurement_seconds * 1000.0;
        let total_cases = config.variant_count();

        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => {}
            Phase::Complete => self.phase = Phase::Complete,
            Phase::Preparing { target_count } => {
                let upload_start = performance_now();
                while renderer.prepared_texture_count() < target_count {
                    if let Err(error) = renderer.prepare_next_texture() {
                        self.fail(error, &mut events);
                        return events;
                    }
                    if performance_now() - upload_start >= 8.0 {
                        break;
                    }
                }
                let prepared = renderer.prepared_texture_count();
                events.push(RunnerEvent::Status(format!(
                    "Uploading distinct {} {}×{} images · {prepared}/{target_count}",
                    self.mode.label(),
                    self.current_size(),
                    self.current_size()
                )));
                events.push(RunnerEvent::UploadProgress {
                    uploaded: prepared,
                    total: target_count,
                });
                if prepared < target_count {
                    self.phase = Phase::Preparing { target_count };
                } else {
                    self.phase = Phase::FinishingUploads;
                }
            }
            Phase::FinishingUploads => {
                renderer.finish_texture_uploads();
                events.push(RunnerEvent::UploadComplete);
                events.extend(self.start_current_case(renderer));
            }
            Phase::Warming {
                mode,
                mut elapsed_ms,
                last_raf,
            } => {
                if let Some(last) = last_raf {
                    let elapsed = now - last;
                    if elapsed.is_finite() && elapsed > 0.0 && elapsed < 500.0 {
                        elapsed_ms += elapsed;
                    }
                }

                if elapsed_ms >= warmup_ms {
                    events.extend(self.start_measurement(mode));
                    self.render_transition_frame(renderer, now, &mut events);
                    return events;
                }

                match renderer.render_once(mode, now) {
                    Ok(_) => {
                        self.phase = Phase::Warming {
                            mode,
                            elapsed_ms,
                            last_raf: Some(now),
                        };
                    }
                    Err(error) => self.fail(error, &mut events),
                }
            }
            Phase::Measuring {
                mode,
                mut frame_intervals,
                mut cpu_times,
                last_raf,
            } => {
                if let Some(last) = last_raf {
                    let elapsed = now - last;
                    if elapsed.is_finite() && elapsed > 0.0 && elapsed < 500.0 {
                        frame_intervals.push(elapsed);
                    } else {
                        frame_intervals.clear();
                    }
                }

                if frame_intervals.iter().sum::<f64>() >= measurement_ms {
                    let metrics = summarize(&frame_intervals, &cpu_times);
                    events.extend(self.finish_measurement(renderer, mode, metrics, total_cases));
                    self.render_transition_frame(renderer, now, &mut events);
                    return events;
                }

                match renderer.render_once(mode, now) {
                    Ok(timing) => {
                        cpu_times.push(timing.cpu_ms);
                        self.phase = Phase::Measuring {
                            mode,
                            frame_intervals,
                            cpu_times,
                            last_raf: Some(now),
                        };
                    }
                    Err(error) => self.fail(error, &mut events),
                }
            }
        }
        events
    }

    fn start_current_case(&mut self, renderer: &mut BenchRenderer) -> Vec<RunnerEvent> {
        let count = self.current_count();
        let draw_size = self.config.as_ref().unwrap().draw_size;
        match renderer.configure_scene(count, draw_size) {
            Ok(()) => self.start_warmup(self.mode),
            Err(error) => {
                self.phase = Phase::Complete;
                vec![RunnerEvent::Failed(error)]
            }
        }
    }

    fn prepare_current_case(&mut self, renderer: &mut BenchRenderer) -> Vec<RunnerEvent> {
        let target_count = self.current_count();
        let uploaded = renderer.prepared_texture_count();
        if uploaded >= target_count {
            return self.start_current_case(renderer);
        }

        self.phase = Phase::Preparing { target_count };
        vec![
            RunnerEvent::Status(format!(
                "Uploading distinct {} {}×{} images · {uploaded}/{target_count}",
                self.mode.label(),
                self.current_size(),
                self.current_size()
            )),
            RunnerEvent::UploadProgress {
                uploaded,
                total: target_count,
            },
        ]
    }

    fn start_warmup(&mut self, mode: RenderMode) -> Vec<RunnerEvent> {
        let warmup_seconds = self.config.as_ref().unwrap().warmup_seconds;
        self.phase = Phase::Warming {
            mode,
            elapsed_ms: 0.0,
            last_raf: None,
        };
        vec![RunnerEvent::Status(format!(
            "Warming {} · {} rects · {warmup_seconds:.1}s",
            mode.label(),
            self.current_count()
        ))]
    }

    fn start_measurement(&mut self, mode: RenderMode) -> Vec<RunnerEvent> {
        let config = self.config.as_ref().unwrap();
        self.phase = Phase::Measuring {
            mode,
            frame_intervals: Vec::with_capacity((config.measurement_seconds * 120.0) as usize),
            cpu_times: Vec::with_capacity((config.measurement_seconds * 120.0) as usize),
            last_raf: None,
        };
        vec![RunnerEvent::Status(format!(
            "Measuring {} · {} rects · {:.1}s",
            mode.label(),
            self.current_count(),
            config.measurement_seconds,
        ))]
    }

    fn finish_measurement(
        &mut self,
        renderer: &mut BenchRenderer,
        mode: RenderMode,
        metrics: MeasurementMetrics,
        configured_case_count: usize,
    ) -> Vec<RunnerEvent> {
        match mode {
            RenderMode::ImagePaint => {
                let stop_early = metrics.fps < 20.0;
                self.image_results.push(metrics);
                if stop_early {
                    self.case_limit = self.count_index + 1;
                }

                let mut events = vec![RunnerEvent::Progress {
                    completed: self.count_index + 1,
                    total: if stop_early {
                        self.case_limit * 2
                    } else {
                        configured_case_count * 2
                    },
                }];

                self.count_index += 1;
                if self.count_index < self.case_limit {
                    events.extend(self.prepare_current_case(renderer));
                    return events;
                }

                self.mode = RenderMode::ExternalTexture;
                self.count_index = 0;
                renderer.begin_benchmark_texture_set(
                    self.current_size(),
                    self.mode,
                    self.image_seed,
                );
                events.extend(self.prepare_current_case(renderer));
                events
            }
            RenderMode::ExternalTexture => {
                let image_metrics = &self.image_results[self.count_index];
                let image_fps = image_metrics.fps;
                let external_fps = metrics.fps;
                let delta_percent = |external: f64, image: f64| {
                    if image > 0.0 {
                        (external - image) / image * 100.0
                    } else {
                        0.0
                    }
                };
                let result = CaseResult {
                    image_count: self.current_count(),
                    image_fps,
                    external_fps,
                    delta_fps_percent: delta_percent(external_fps, image_fps),
                    image_cpu_ms: image_metrics.cpu_ms,
                    external_cpu_ms: metrics.cpu_ms,
                    delta_cpu_percent: delta_percent(metrics.cpu_ms, image_metrics.cpu_ms),
                };
                let stop_early = external_fps < 20.0;
                let mut events = vec![
                    RunnerEvent::CaseComplete(result),
                    RunnerEvent::Progress {
                        completed: self.case_limit + self.count_index + 1,
                        total: self.case_limit * 2,
                    },
                ];

                if stop_early {
                    let count = self.current_count();
                    self.phase = Phase::Complete;
                    renderer.delete_textures();
                    events.push(RunnerEvent::Complete(format!(
                        "Stopped after {count} rects because average FPS fell below 20"
                    )));
                    return events;
                }

                self.count_index += 1;
                if self.count_index < self.case_limit {
                    events.extend(self.prepare_current_case(renderer));
                    return events;
                }

                self.phase = Phase::Complete;
                renderer.delete_textures();
                events.push(RunnerEvent::Complete("Benchmark complete".to_string()));
                events
            }
        }
    }

    fn fail(&mut self, error: String, events: &mut Vec<RunnerEvent>) {
        self.phase = Phase::Complete;
        events.push(RunnerEvent::Failed(error));
    }

    /// Render immediately after switching into a new warmup or measurement phase so the phase
    /// transition does not consume an animation frame without presenting updated content.
    fn render_transition_frame(
        &mut self,
        renderer: &mut BenchRenderer,
        now: f64,
        events: &mut Vec<RunnerEvent>,
    ) {
        let mode = match &self.phase {
            Phase::Warming { mode, .. } | Phase::Measuring { mode, .. } => *mode,
            _ => return,
        };

        if let Err(error) = renderer.render_once(mode, now) {
            self.fail(error, events);
            return;
        }

        match &mut self.phase {
            Phase::Warming { last_raf, .. } | Phase::Measuring { last_raf, .. } => {
                *last_raf = Some(now);
            }
            _ => {}
        }
    }

    fn current_size(&self) -> u16 {
        self.config.as_ref().unwrap().texture_size
    }

    fn current_count(&self) -> usize {
        self.config.as_ref().unwrap().image_counts[self.count_index]
    }
}

fn summarize(frame_intervals: &[f64], cpu_times: &[f64]) -> MeasurementMetrics {
    let elapsed_ms = frame_intervals.iter().sum::<f64>();
    MeasurementMetrics {
        fps: frame_intervals.len() as f64 * 1000.0 / elapsed_ms,
        cpu_ms: mean(cpu_times).unwrap_or(0.0),
    }
}

fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn performance_now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

#[cfg(test)]
mod tests {
    use super::summarize;

    #[test]
    fn fps_uses_the_full_measurement_interval() {
        let metrics = summarize(&[10.0, 20.0, 20.0], &[2.0, 4.0]);
        assert_eq!(metrics.fps, 60.0);
        assert_eq!(metrics.cpu_ms, 3.0);
    }
}

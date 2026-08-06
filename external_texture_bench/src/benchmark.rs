use crate::renderer::{BenchRenderer, RenderMode};

#[derive(Clone, Debug)]
pub struct BenchConfig {
    pub image_counts: Vec<usize>,
    pub texture_size: u16,
    pub texture_count: usize,
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
        if self.texture_count < 2 {
            return Err(
                "use at least 2 textures so adjacent images cannot merge into one run".to_string(),
            );
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
}

#[derive(Clone, Debug)]
pub struct CaseResult {
    pub image_count: usize,
    pub image_fps: f64,
    pub external_fps: f64,
    pub delta_fps_percent: f64,
}

#[derive(Debug)]
pub enum RunnerEvent {
    Status(String),
    Progress { completed: usize, total: usize },
    CaseComplete(CaseResult),
    Complete(String),
    Failed(String),
}

#[derive(Debug)]
enum Phase {
    Idle,
    Preparing {
        next_texture: usize,
    },
    Warming {
        mode: RenderMode,
        elapsed_ms: f64,
        last_raf: Option<f64>,
    },
    Measuring {
        mode: RenderMode,
        frame_intervals: Vec<f64>,
        last_raf: Option<f64>,
    },
    Complete,
}

pub struct BenchRunner {
    config: Option<BenchConfig>,
    phase: Phase,
    count_index: usize,
    pending_image: Option<MeasurementMetrics>,
    pending_external: Option<MeasurementMetrics>,
}

impl BenchRunner {
    pub fn new() -> Self {
        Self {
            config: None,
            phase: Phase::Idle,
            count_index: 0,
            pending_image: None,
            pending_external: None,
        }
    }

    pub fn start(&mut self, config: BenchConfig, renderer: &mut BenchRenderer) -> Vec<RunnerEvent> {
        let size = config.texture_size;
        let texture_count = config.texture_count;
        self.config = Some(config);
        self.count_index = 0;
        self.pending_image = None;
        self.pending_external = None;
        renderer.begin_texture_set(size);
        self.phase = Phase::Preparing { next_texture: 0 };
        vec![RunnerEvent::Status(format!(
            "Preparing {texture_count} paired textures at {size}×{size}"
        ))]
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
                last_raf,
                ..
            } => {
                frame_intervals.clear();
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
        let texture_count = config.texture_count;
        let warmup_ms = config.warmup_seconds * 1000.0;
        let measurement_ms = config.measurement_seconds * 1000.0;
        let total_cases = config.variant_count();

        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => {}
            Phase::Complete => self.phase = Phase::Complete,
            Phase::Preparing { next_texture } => {
                if let Err(error) = renderer.prepare_next_texture() {
                    self.fail(error, &mut events);
                    return events;
                }
                let prepared = next_texture + 1;
                let required = texture_count;
                events.push(RunnerEvent::Status(format!(
                    "Preparing paired {}×{} textures · {prepared}/{required}",
                    self.current_size(),
                    self.current_size()
                )));
                if prepared < required {
                    self.phase = Phase::Preparing {
                        next_texture: prepared,
                    };
                } else {
                    events.extend(self.start_current_case(renderer));
                }
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
                    if mode == RenderMode::ImagePaint {
                        events.extend(self.start_warmup(RenderMode::ExternalTexture));
                    } else {
                        events.extend(self.start_measurement_pair());
                    }
                    return events;
                }

                match renderer.render_once(mode, now) {
                    Ok(()) => {
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
                    let metrics = summarize(&frame_intervals);
                    match mode {
                        RenderMode::ImagePaint => self.pending_image = Some(metrics),
                        RenderMode::ExternalTexture => self.pending_external = Some(metrics),
                    }

                    if self.pending_image.is_some() && self.pending_external.is_some() {
                        let result = self.finish_pair();
                        let stop_early = result.image_fps < 20.0 || result.external_fps < 20.0;
                        events.push(RunnerEvent::CaseComplete(result));
                        events.push(RunnerEvent::Progress {
                            completed: self.count_index + 1,
                            total: total_cases,
                        });
                        events.extend(self.advance_after_case(renderer, stop_early));
                    } else {
                        events.extend(self.start_measurement(mode.other()));
                    }
                    return events;
                }

                match renderer.render_once(mode, now) {
                    Ok(()) => {
                        self.phase = Phase::Measuring {
                            mode,
                            frame_intervals,
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
            Ok(()) => self.start_warmup(RenderMode::ImagePaint),
            Err(error) => {
                self.phase = Phase::Complete;
                vec![RunnerEvent::Failed(error)]
            }
        }
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

    fn start_measurement_pair(&mut self) -> Vec<RunnerEvent> {
        self.pending_image = None;
        self.pending_external = None;
        let first = if self.count_index.is_multiple_of(2) {
            RenderMode::ImagePaint
        } else {
            RenderMode::ExternalTexture
        };
        self.start_measurement(first)
    }

    fn start_measurement(&mut self, mode: RenderMode) -> Vec<RunnerEvent> {
        let config = self.config.as_ref().unwrap();
        self.phase = Phase::Measuring {
            mode,
            frame_intervals: Vec::with_capacity((config.measurement_seconds * 120.0) as usize),
            last_raf: None,
        };
        vec![RunnerEvent::Status(format!(
            "Measuring {} · {} rects · {:.1}s",
            mode.label(),
            self.current_count(),
            config.measurement_seconds,
        ))]
    }

    fn finish_pair(&mut self) -> CaseResult {
        let image_paint = self.pending_image.take().unwrap();
        let external_texture = self.pending_external.take().unwrap();
        CaseResult {
            image_count: self.current_count(),
            image_fps: image_paint.fps,
            external_fps: external_texture.fps,
            delta_fps_percent: if image_paint.fps > 0.0 {
                (external_texture.fps - image_paint.fps) / image_paint.fps * 100.0
            } else {
                0.0
            },
        }
    }

    fn advance_after_case(
        &mut self,
        renderer: &mut BenchRenderer,
        stop_early: bool,
    ) -> Vec<RunnerEvent> {
        if stop_early {
            let count = self.current_count();
            self.phase = Phase::Complete;
            renderer.delete_textures();
            return vec![RunnerEvent::Complete(format!(
                "Stopped after {count} images because average FPS fell below 20"
            ))];
        }

        self.count_index += 1;
        if self.count_index < self.config.as_ref().unwrap().image_counts.len() {
            return self.start_current_case(renderer);
        }

        self.phase = Phase::Complete;
        renderer.delete_textures();
        vec![RunnerEvent::Complete("Benchmark complete".to_string())]
    }

    fn fail(&mut self, error: String, events: &mut Vec<RunnerEvent>) {
        self.phase = Phase::Complete;
        events.push(RunnerEvent::Failed(error));
    }

    fn current_size(&self) -> u16 {
        self.config.as_ref().unwrap().texture_size
    }

    fn current_count(&self) -> usize {
        self.config.as_ref().unwrap().image_counts[self.count_index]
    }
}

fn summarize(frame_intervals: &[f64]) -> MeasurementMetrics {
    let elapsed_ms = frame_intervals.iter().sum::<f64>();
    MeasurementMetrics {
        fps: frame_intervals.len() as f64 * 1000.0 / elapsed_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::summarize;

    #[test]
    fn fps_uses_the_full_measurement_interval() {
        let metrics = summarize(&[10.0, 20.0, 20.0]);
        assert_eq!(metrics.fps, 60.0);
    }
}

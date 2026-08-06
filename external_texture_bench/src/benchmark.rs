use crate::renderer::{BenchRenderer, RenderMode};

#[derive(Clone, Debug)]
pub struct BenchConfig {
    pub image_counts: Vec<usize>,
    pub texture_size: u16,
    pub texture_count: usize,
    pub draw_size: u16,
    pub warmup_frames: usize,
    pub measured_frames: usize,
    pub trials: usize,
    pub memory_limit_mib: usize,
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
        if self.warmup_frames == 0 || self.measured_frames < 10 || self.trials == 0 {
            return Err(
                "use at least 1 warmup frame, 10 measured frames, and 1 measurement".to_string(),
            );
        }
        if self.texture_set_bytes() > self.memory_limit_bytes() {
            return Err(format!(
                "the paired texture sets need {} MiB, exceeding the configured {} MiB limit",
                self.texture_set_bytes().div_ceil(1024 * 1024),
                self.memory_limit_mib
            ));
        }
        Ok(())
    }

    pub fn variant_count(&self) -> usize {
        self.image_counts.len()
    }

    pub fn texture_set_bytes(&self) -> u64 {
        let side = u64::from(self.texture_size);
        // The same pixels are resident once in Vello's image atlas and once in standalone
        // external textures so the two strategies can be compared without uploads in timing.
        side * side * 4 * self.texture_count as u64 * 2
    }

    pub fn memory_limit_bytes(&self) -> u64 {
        self.memory_limit_mib as u64 * 1024 * 1024
    }
}

#[derive(Clone, Debug)]
pub struct MeasurementMetrics {
    pub median_frame_ms: f64,
    pub mean_frame_ms: f64,
    pub p95_frame_ms: f64,
    pub fps: f64,
    pub median_cpu_submit_ms: f64,
}

#[derive(Clone, Debug)]
pub struct TrialResult {
    pub texture_size: u16,
    pub texture_count: usize,
    pub image_count: usize,
    pub measurement: usize,
    pub image_paint: MeasurementMetrics,
    pub external_texture: MeasurementMetrics,
    pub delta_frame_ms: f64,
    pub delta_frame_percent: f64,
    pub delta_cpu_submit_ms: f64,
}

#[derive(Debug)]
pub enum RunnerEvent {
    Status(String),
    Progress { completed: usize, total: usize },
    TrialComplete(TrialResult),
    Complete,
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
        remaining: usize,
    },
    Measuring {
        mode: RenderMode,
        frame_intervals: Vec<f64>,
        cpu_submit_times: Vec<f64>,
        last_raf: Option<f64>,
        pending_cpu_ms: Option<f64>,
    },
    Complete,
}

pub struct BenchRunner {
    config: Option<BenchConfig>,
    phase: Phase,
    count_index: usize,
    trial_index: usize,
    completed_trials: usize,
    pending_image: Option<MeasurementMetrics>,
    pending_external: Option<MeasurementMetrics>,
}

impl BenchRunner {
    pub fn new() -> Self {
        Self {
            config: None,
            phase: Phase::Idle,
            count_index: 0,
            trial_index: 0,
            completed_trials: 0,
            pending_image: None,
            pending_external: None,
        }
    }

    pub fn start(&mut self, config: BenchConfig, renderer: &mut BenchRenderer) -> Vec<RunnerEvent> {
        let size = config.texture_size;
        let texture_count = config.texture_count;
        self.config = Some(config);
        self.count_index = 0;
        self.trial_index = 0;
        self.completed_trials = 0;
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
        if let Phase::Measuring {
            frame_intervals,
            cpu_submit_times,
            last_raf,
            pending_cpu_ms,
            ..
        } = &mut self.phase
        {
            frame_intervals.clear();
            cpu_submit_times.clear();
            *last_raf = None;
            *pending_cpu_ms = None;
        }
    }

    /// Advance exactly one browser animation frame and submit exactly one Vello render.
    pub fn tick(&mut self, renderer: &mut BenchRenderer, now: f64) -> Vec<RunnerEvent> {
        let mut events = Vec::new();
        let Some(config) = self.config.as_ref() else {
            return events;
        };
        let texture_count = config.texture_count;
        let measured_frames = config.measured_frames;
        let total_trials = config.variant_count() * config.trials;

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
                mut remaining,
            } => match renderer.render_once(mode, now) {
                Ok(_) => {
                    remaining = remaining.saturating_sub(1);
                    if remaining > 0 {
                        self.phase = Phase::Warming { mode, remaining };
                    } else if mode == RenderMode::ImagePaint {
                        events.extend(self.start_warmup(RenderMode::ExternalTexture));
                    } else {
                        events.extend(self.start_measurement_pair());
                    }
                }
                Err(error) => self.fail(error, &mut events),
            },
            Phase::Measuring {
                mode,
                mut frame_intervals,
                mut cpu_submit_times,
                last_raf,
                pending_cpu_ms,
            } => {
                if let (Some(last), Some(cpu_ms)) = (last_raf, pending_cpu_ms) {
                    let elapsed = now - last;
                    if elapsed.is_finite() && elapsed > 0.0 && elapsed < 500.0 {
                        frame_intervals.push(elapsed);
                        cpu_submit_times.push(cpu_ms);
                    }
                }

                if frame_intervals.len() >= measured_frames {
                    let metrics = summarize(frame_intervals, cpu_submit_times);
                    match mode {
                        RenderMode::ImagePaint => self.pending_image = Some(metrics),
                        RenderMode::ExternalTexture => self.pending_external = Some(metrics),
                    }

                    if self.pending_image.is_some() && self.pending_external.is_some() {
                        let result = self.finish_pair();
                        self.completed_trials += 1;
                        events.push(RunnerEvent::TrialComplete(result));
                        events.push(RunnerEvent::Progress {
                            completed: self.completed_trials,
                            total: total_trials,
                        });
                        events.extend(self.advance_after_trial(renderer));
                    } else {
                        events.extend(self.start_measurement(mode.other()));
                    }
                    return events;
                }

                match renderer.render_once(mode, now) {
                    Ok(cpu_ms) => {
                        self.phase = Phase::Measuring {
                            mode,
                            frame_intervals,
                            cpu_submit_times,
                            last_raf: Some(now),
                            pending_cpu_ms: Some(cpu_ms),
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
        let warmup_frames = self.config.as_ref().unwrap().warmup_frames;
        self.phase = Phase::Warming {
            mode,
            remaining: warmup_frames,
        };
        vec![RunnerEvent::Status(format!(
            "Warming {} · {} images · {warmup_frames} frames",
            mode.label(),
            self.current_count()
        ))]
    }

    fn start_measurement_pair(&mut self) -> Vec<RunnerEvent> {
        self.pending_image = None;
        self.pending_external = None;
        let first = if self.trial_index.is_multiple_of(2) {
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
            frame_intervals: Vec::with_capacity(config.measured_frames),
            cpu_submit_times: Vec::with_capacity(config.measured_frames),
            last_raf: None,
            pending_cpu_ms: None,
        };
        vec![RunnerEvent::Status(format!(
            "Measuring {} · {} images · pair {}/{}",
            mode.label(),
            self.current_count(),
            self.trial_index + 1,
            config.trials
        ))]
    }

    fn finish_pair(&mut self) -> TrialResult {
        let image_paint = self.pending_image.take().unwrap();
        let external_texture = self.pending_external.take().unwrap();
        let delta_frame_ms = external_texture.median_frame_ms - image_paint.median_frame_ms;
        let delta_frame_percent = if image_paint.median_frame_ms > 0.0 {
            delta_frame_ms / image_paint.median_frame_ms * 100.0
        } else {
            0.0
        };
        let delta_cpu_submit_ms =
            external_texture.median_cpu_submit_ms - image_paint.median_cpu_submit_ms;
        TrialResult {
            texture_size: self.current_size(),
            texture_count: self.config.as_ref().unwrap().texture_count,
            image_count: self.current_count(),
            measurement: self.trial_index + 1,
            image_paint,
            external_texture,
            delta_frame_ms,
            delta_frame_percent,
            delta_cpu_submit_ms,
        }
    }

    fn advance_after_trial(&mut self, renderer: &mut BenchRenderer) -> Vec<RunnerEvent> {
        let config = self.config.as_ref().unwrap();
        self.trial_index += 1;
        if self.trial_index < config.trials {
            return self.start_measurement_pair();
        }

        self.trial_index = 0;
        self.count_index += 1;
        if self.count_index < config.image_counts.len() {
            return self.start_current_case(renderer);
        }

        self.phase = Phase::Complete;
        renderer.delete_textures();
        vec![RunnerEvent::Complete]
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

fn summarize(frame_intervals: Vec<f64>, cpu_submit_times: Vec<f64>) -> MeasurementMetrics {
    let median_frame_ms = median(&frame_intervals);
    MeasurementMetrics {
        median_frame_ms,
        mean_frame_ms: mean(&frame_intervals),
        p95_frame_ms: percentile(&frame_intervals, 0.95),
        fps: 1000.0 / median_frame_ms,
        median_cpu_submit_ms: median(&cpu_submit_times),
    }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len().max(1) as f64
}

fn median(values: &[f64]) -> f64 {
    percentile(values, 0.5)
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() - 1) as f64 * percentile).round() as usize;
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::{mean, median, percentile};

    #[test]
    fn summary_statistics_are_stable() {
        let values = [3.0, 1.0, 4.0, 2.0, 5.0];
        assert_eq!(median(&values), 3.0);
        assert_eq!(mean(&values), 3.0);
        assert_eq!(percentile(&values, 0.95), 5.0);
    }
}

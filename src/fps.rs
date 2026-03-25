//! Frame timing averaged over 1 second.

/// Tracks wall-clock frame time, reporting averages over 1-second windows.
#[derive(Debug)]
pub(crate) struct FpsTracker {
    last_time: f64,
    window_start: f64,
    frame_count: u32,
    /// Cached results from the last completed 1-second window.
    avg_fps: f64,
    avg_frame_time: f64,
}

impl FpsTracker {
    /// Create a new tracker starting at the given timestamp (ms).
    pub(crate) fn new(now: f64) -> Self {
        Self {
            last_time: now,
            window_start: now,
            frame_count: 0,
            avg_fps: 0.0,
            avg_frame_time: 0.0,
        }
    }

    /// Record a frame. `now` is the current `performance.now()` timestamp.
    ///
    /// Returns `(fps, avg_frame_time_ms)` averaged over 1-second windows.
    pub(crate) fn frame(&mut self, now: f64) -> (f64, f64) {
        self.last_time = now;
        self.frame_count += 1;

        let elapsed = now - self.window_start;
        if elapsed >= 1000.0 {
            self.avg_frame_time = elapsed / self.frame_count as f64;
            self.avg_fps = self.frame_count as f64 * 1000.0 / elapsed;
            self.window_start = now;
            self.frame_count = 0;
        }

        (self.avg_fps, self.avg_frame_time)
    }
}

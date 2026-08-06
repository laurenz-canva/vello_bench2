use crate::benchmark::{BenchConfig, RunnerEvent, TrialResult};
use wasm_bindgen::JsCast;
use web_sys::{Document, HtmlButtonElement, HtmlElement, HtmlInputElement};

pub struct Ui {
    image_counts: HtmlInputElement,
    texture_size: HtmlInputElement,
    texture_count: HtmlInputElement,
    draw_size: HtmlInputElement,
    warmup_frames: HtmlInputElement,
    measured_frames: HtmlInputElement,
    trials: HtmlInputElement,
    memory_limit_mib: HtmlInputElement,
    run_button: HtmlButtonElement,
    stop_button: HtmlButtonElement,
    restart_button: HtmlButtonElement,
    setup_screen: HtmlElement,
    running_screen: HtmlElement,
    results_screen: HtmlElement,
    config_error: HtmlElement,
    running_status: HtmlElement,
    results_status: HtmlElement,
    progress_fill: HtmlElement,
    results_body: HtmlElement,
    device_info: HtmlElement,
}

impl Ui {
    pub fn new(document: &Document) -> Result<Self, String> {
        Ok(Self {
            image_counts: element(document, "image-counts")?,
            texture_size: element(document, "texture-size")?,
            texture_count: element(document, "texture-count")?,
            draw_size: element(document, "draw-size")?,
            warmup_frames: element(document, "warmup-frames")?,
            measured_frames: element(document, "measured-frames")?,
            trials: element(document, "trials")?,
            memory_limit_mib: element(document, "memory-limit-mib")?,
            run_button: element(document, "run-button")?,
            stop_button: element(document, "stop-button")?,
            restart_button: element(document, "restart-button")?,
            setup_screen: element(document, "setup-screen")?,
            running_screen: element(document, "running-screen")?,
            results_screen: element(document, "results-screen")?,
            config_error: element(document, "config-error")?,
            running_status: element(document, "running-status")?,
            results_status: element(document, "results-status")?,
            progress_fill: element(document, "progress-fill")?,
            results_body: element(document, "results-body")?,
            device_info: element(document, "device-info")?,
        })
    }

    pub fn run_button(&self) -> HtmlButtonElement {
        self.run_button.clone()
    }

    pub fn stop_button(&self) -> HtmlButtonElement {
        self.stop_button.clone()
    }

    pub fn restart_button(&self) -> HtmlButtonElement {
        self.restart_button.clone()
    }

    pub fn read_config(&self) -> Result<BenchConfig, String> {
        Ok(BenchConfig {
            image_counts: parse_list::<usize>(&self.image_counts.value(), "image counts")?,
            texture_size: parse_value(&self.texture_size, "texture size")?,
            texture_count: parse_value(&self.texture_count, "texture count")?,
            draw_size: parse_value(&self.draw_size, "draw size")?,
            warmup_frames: parse_value(&self.warmup_frames, "warmup frames")?,
            measured_frames: parse_value(&self.measured_frames, "measured frames")?,
            trials: parse_value(&self.trials, "measurements")?,
            memory_limit_mib: parse_value(&self.memory_limit_mib, "memory limit")?,
        })
    }

    pub fn begin_run(&self) {
        self.results_body.set_inner_html("");
        self.config_error.set_text_content(None);
        self.results_status.set_text_content(None);
        self.set_progress(0, 1);
        set_visible(&self.setup_screen, false);
        set_visible(&self.results_screen, false);
        set_visible(&self.running_screen, true);
        self.set_status("Preparing benchmark…");
    }

    pub fn show_setup(&self, message: Option<&str>) {
        set_visible(&self.running_screen, false);
        set_visible(&self.results_screen, false);
        set_visible(&self.setup_screen, true);
        self.config_error.set_text_content(message);
    }

    pub fn finish_run(&self, message: &str) {
        set_visible(&self.running_screen, false);
        set_visible(&self.setup_screen, false);
        set_visible(&self.results_screen, true);
        self.results_status.set_text_content(Some(message));
    }

    pub fn set_status(&self, message: &str) {
        self.running_status.set_text_content(Some(message));
    }

    pub fn set_device_info(&self, max_texture_size: u32) {
        self.device_info.set_text_content(Some(&format!(
            "WebGL2 · {}×{} render target · max texture {}px · Vello badbde90",
            crate::renderer::CANVAS_SIZE,
            crate::renderer::CANVAS_SIZE,
            max_texture_size
        )));
    }

    pub fn handle_event(&self, event: RunnerEvent) {
        match event {
            RunnerEvent::Status(message) => self.set_status(&message),
            RunnerEvent::Progress { completed, total } => self.set_progress(completed, total),
            RunnerEvent::TrialComplete(result) => self.append_result(&result),
            RunnerEvent::Complete => self.finish_run("Benchmark complete"),
            RunnerEvent::Failed(error) => self.finish_run(&format!("Benchmark failed: {error}")),
        }
    }

    fn set_progress(&self, completed: usize, total: usize) {
        let percentage = completed as f64 / total.max(1) as f64 * 100.0;
        let _ = self
            .progress_fill
            .style()
            .set_property("width", &format!("{percentage:.2}%"));
    }

    fn append_result(&self, result: &TrialResult) {
        let document = self.results_body.owner_document().unwrap();
        let row = document.create_element("tr").unwrap();
        for (label, value) in [
            (
                "Texture",
                format!("{}×{}", result.texture_size, result.texture_size),
            ),
            ("Textures", result.texture_count.to_string()),
            ("Images", result.image_count.to_string()),
            ("Measurement", result.measurement.to_string()),
            ("Image FPS", format!("{:.2}", result.image_paint.fps)),
            (
                "External FPS",
                format!("{:.2}", result.external_texture.fps),
            ),
            (
                "Image frame ms",
                format!("{:.2}", result.image_paint.median_frame_ms),
            ),
            (
                "External frame ms",
                format!("{:.2}", result.external_texture.median_frame_ms),
            ),
            (
                "Image mean ms",
                format!("{:.2}", result.image_paint.mean_frame_ms),
            ),
            (
                "External mean ms",
                format!("{:.2}", result.external_texture.mean_frame_ms),
            ),
            ("Δ frame ms", format_delta(result.delta_frame_ms, 3)),
            ("Δ frame %", format_delta(result.delta_frame_percent, 2)),
            (
                "Image CPU ms",
                format!("{:.3}", result.image_paint.median_cpu_submit_ms),
            ),
            (
                "External CPU ms",
                format!("{:.3}", result.external_texture.median_cpu_submit_ms),
            ),
            ("Δ CPU ms", format_delta(result.delta_cpu_submit_ms, 3)),
            (
                "Image p95 ms",
                format!("{:.2}", result.image_paint.p95_frame_ms),
            ),
            (
                "External p95 ms",
                format!("{:.2}", result.external_texture.p95_frame_ms),
            ),
        ] {
            let cell = document.create_element("td").unwrap();
            cell.set_attribute("data-label", label).unwrap();
            cell.set_text_content(Some(&value));
            row.append_child(&cell).unwrap();
        }
        self.results_body.append_child(&row).unwrap();
    }
}

fn format_delta(value: f64, precision: usize) -> String {
    format!("{value:+.*}", precision)
}

fn set_visible(element: &HtmlElement, visible: bool) {
    if visible {
        let _ = element.remove_attribute("hidden");
    } else {
        let _ = element.set_attribute("hidden", "");
    }
}

fn element<T: JsCast>(document: &Document, id: &str) -> Result<T, String> {
    document
        .get_element_by_id(id)
        .ok_or_else(|| format!("missing #{id}"))?
        .dyn_into::<T>()
        .map_err(|_| format!("#{id} has the wrong element type"))
}

fn parse_list<T>(text: &str, label: &str) -> Result<Vec<T>, String>
where
    T: std::str::FromStr,
{
    text.split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<T>()
                .map_err(|_| format!("invalid {label} value: {part}"))
        })
        .collect()
}

fn parse_value<T>(input: &HtmlInputElement, label: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    input
        .value()
        .parse::<T>()
        .map_err(|_| format!("invalid {label}"))
}

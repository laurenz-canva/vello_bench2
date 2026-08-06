use crate::benchmark::{BenchConfig, CaseResult, RunnerEvent};
use crate::renderer::RenderMode;
use wasm_bindgen::JsCast;
use web_sys::{Document, HtmlButtonElement, HtmlElement, HtmlInputElement, HtmlSelectElement};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractiveConfig {
    pub image_count: usize,
    pub texture_size: u16,
    pub texture_count: usize,
    pub draw_size: u16,
    pub mode: RenderMode,
}

pub struct Ui {
    image_counts: HtmlInputElement,
    texture_size: HtmlInputElement,
    texture_count: HtmlInputElement,
    draw_size: HtmlInputElement,
    warmup_seconds: HtmlInputElement,
    measurement_seconds: HtmlInputElement,
    run_button: HtmlButtonElement,
    interactive_button: HtmlButtonElement,
    stop_button: HtmlButtonElement,
    interactive_back_button: HtmlButtonElement,
    restart_button: HtmlButtonElement,
    setup_screen: HtmlElement,
    running_screen: HtmlElement,
    results_screen: HtmlElement,
    config_error: HtmlElement,
    running_status: HtmlElement,
    results_status: HtmlElement,
    progress_fill: HtmlElement,
    benchmark_overlay: HtmlElement,
    interactive_overlay: HtmlElement,
    progress: HtmlElement,
    results_body: HtmlElement,
    device_info: HtmlElement,
    interactive_image_count: HtmlInputElement,
    interactive_texture_size: HtmlInputElement,
    interactive_texture_count: HtmlInputElement,
    interactive_draw_size: HtmlInputElement,
    interactive_source: HtmlSelectElement,
    interactive_status: HtmlElement,
    interactive_error: HtmlElement,
}

impl Ui {
    pub fn new(document: &Document) -> Result<Self, String> {
        Ok(Self {
            image_counts: element(document, "image-counts")?,
            texture_size: element(document, "texture-size")?,
            texture_count: element(document, "texture-count")?,
            draw_size: element(document, "draw-size")?,
            warmup_seconds: element(document, "warmup-seconds")?,
            measurement_seconds: element(document, "measurement-seconds")?,
            run_button: element(document, "run-button")?,
            interactive_button: element(document, "interactive-button")?,
            stop_button: element(document, "stop-button")?,
            interactive_back_button: element(document, "interactive-back-button")?,
            restart_button: element(document, "restart-button")?,
            setup_screen: element(document, "setup-screen")?,
            running_screen: element(document, "running-screen")?,
            results_screen: element(document, "results-screen")?,
            config_error: element(document, "config-error")?,
            running_status: element(document, "running-status")?,
            results_status: element(document, "results-status")?,
            progress_fill: element(document, "progress-fill")?,
            benchmark_overlay: element(document, "benchmark-overlay")?,
            interactive_overlay: element(document, "interactive-overlay")?,
            progress: element(document, "progress")?,
            results_body: element(document, "results-body")?,
            device_info: element(document, "device-info")?,
            interactive_image_count: element(document, "interactive-image-count")?,
            interactive_texture_size: element(document, "interactive-texture-size")?,
            interactive_texture_count: element(document, "interactive-texture-count")?,
            interactive_draw_size: element(document, "interactive-draw-size")?,
            interactive_source: element(document, "interactive-source")?,
            interactive_status: element(document, "interactive-status")?,
            interactive_error: element(document, "interactive-error")?,
        })
    }

    pub fn run_button(&self) -> HtmlButtonElement {
        self.run_button.clone()
    }

    pub fn stop_button(&self) -> HtmlButtonElement {
        self.stop_button.clone()
    }

    pub fn interactive_button(&self) -> HtmlButtonElement {
        self.interactive_button.clone()
    }

    pub fn interactive_back_button(&self) -> HtmlButtonElement {
        self.interactive_back_button.clone()
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
            warmup_seconds: parse_value(&self.warmup_seconds, "warmup duration")?,
            measurement_seconds: parse_value(&self.measurement_seconds, "measurement duration")?,
        })
    }

    pub fn read_interactive_config(
        &self,
        max_texture_size: u32,
    ) -> Result<InteractiveConfig, String> {
        let config = InteractiveConfig {
            image_count: parse_value(&self.interactive_image_count, "image count")?,
            texture_size: parse_value(&self.interactive_texture_size, "texture size")?,
            texture_count: parse_value(&self.interactive_texture_count, "texture count")?,
            draw_size: parse_value(&self.interactive_draw_size, "draw size")?,
            mode: match self.interactive_source.value().as_str() {
                "image" => RenderMode::ImagePaint,
                "external" => RenderMode::ExternalTexture,
                _ => return Err("invalid image source".to_string()),
            },
        };
        if config.image_count == 0 {
            return Err("image count must be greater than zero".to_string());
        }
        if config.texture_count == 0 {
            return Err("resident textures must be greater than zero".to_string());
        }
        if config.texture_size < 4 || u32::from(config.texture_size) > max_texture_size {
            return Err(format!(
                "texture size must be between 4 and {max_texture_size}px"
            ));
        }
        if config.draw_size == 0 {
            return Err("draw size must be greater than zero".to_string());
        }
        Ok(config)
    }

    pub fn begin_run(&self) {
        self.results_body.set_inner_html("");
        self.config_error.set_text_content(None);
        self.results_status.set_text_content(None);
        self.set_progress(0, 1);
        set_visible(&self.setup_screen, false);
        set_visible(&self.results_screen, false);
        set_visible(&self.running_screen, true);
        set_visible(&self.benchmark_overlay, true);
        set_visible(&self.interactive_overlay, false);
        set_visible(&self.progress, true);
        self.set_status("Preparing benchmark…");
    }

    pub fn begin_interactive(&self) {
        self.config_error.set_text_content(None);
        set_visible(&self.setup_screen, false);
        set_visible(&self.results_screen, false);
        set_visible(&self.running_screen, true);
        set_visible(&self.benchmark_overlay, false);
        set_visible(&self.interactive_overlay, true);
        set_visible(&self.progress, false);
        self.interactive_error.set_text_content(None);
        self.set_interactive_status("Preparing textures…");
    }

    pub fn show_setup(&self, message: Option<&str>) {
        set_visible(&self.running_screen, false);
        set_visible(&self.results_screen, false);
        set_visible(&self.setup_screen, true);
        self.config_error.set_text_content(message);
    }

    pub fn set_interactive_status(&self, message: &str) {
        self.interactive_status.set_text_content(Some(message));
    }

    pub fn set_interactive_error(&self, message: Option<&str>) {
        self.interactive_error.set_text_content(message);
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

    pub fn set_device_info(&self, max_texture_size: u32, viewport: (u16, u16)) {
        self.device_info.set_text_content(Some(&format!(
            "WebGL2 · {}×{} render target · max texture {}px · Vello badbde90",
            viewport.0, viewport.1, max_texture_size
        )));
    }

    pub fn handle_event(&self, event: RunnerEvent) {
        match event {
            RunnerEvent::Status(message) => self.set_status(&message),
            RunnerEvent::Progress { completed, total } => self.set_progress(completed, total),
            RunnerEvent::CaseComplete(result) => self.append_result(&result),
            RunnerEvent::Complete(message) => {
                self.set_progress(1, 1);
                self.finish_run(&message);
            }
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

    fn append_result(&self, result: &CaseResult) {
        let document = self.results_body.owner_document().unwrap();
        let row = document.create_element("tr").unwrap();
        let result_class = if result.delta_fps_percent >= 10.0 {
            "faster"
        } else if result.delta_fps_percent <= -10.0 {
            "slower"
        } else {
            "neutral"
        };
        row.set_class_name(result_class);
        for (label, value) in [
            ("Rects", result.image_count.to_string()),
            ("Image FPS", format!("{:.1}", result.image_fps)),
            ("External FPS", format!("{:.1}", result.external_fps)),
            ("Difference", format_delta(result.delta_fps_percent, 1)),
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
    let rounded_zero_threshold = 0.5 * 10_f64.powi(-(precision as i32));
    let value = if value.abs() < rounded_zero_threshold {
        0.0
    } else {
        value
    };
    format!("{value:+.*}%", precision)
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

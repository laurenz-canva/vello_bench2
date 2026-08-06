mod benchmark;
mod renderer;
mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use benchmark::BenchRunner;
use renderer::BenchRenderer;
use ui::{InteractiveConfig, Ui};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

struct App {
    renderer: BenchRenderer,
    runner: BenchRunner,
    ui: Ui,
    interactive: Option<InteractiveSession>,
}

struct InteractiveSession {
    config: InteractiveConfig,
    prepared_textures: usize,
    configured_scene: Option<(usize, u16)>,
    last_raf: Option<f64>,
    smoothed_fps: Option<f64>,
}

impl App {
    fn tick(&mut self, now: f64) {
        let hidden = web_sys::window()
            .and_then(|window| window.document())
            .is_some_and(|document| document.hidden());
        if hidden {
            if let Some(interactive) = &mut self.interactive {
                interactive.last_raf = None;
                interactive.smoothed_fps = None;
            } else {
                self.runner.invalidate_pending_timing();
            }
            return;
        }

        if self.interactive.is_some() {
            self.tick_interactive(now);
            return;
        }

        let events = self.runner.tick(&mut self.renderer, now);
        for event in events {
            self.ui.handle_event(event);
        }
    }

    fn start(&mut self) {
        self.interactive = None;
        let max_texture_size = self.renderer.max_texture_size();
        let mut config = match self.ui.read_config() {
            Ok(config) => config,
            Err(error) => {
                self.ui
                    .show_setup(Some(&format!("Invalid configuration: {error}")));
                return;
            }
        };
        if let Err(error) = config.validate(max_texture_size) {
            self.ui
                .show_setup(Some(&format!("Invalid configuration: {error}")));
            return;
        }

        self.ui.begin_run();
        let events = self.runner.start(config, &mut self.renderer);
        for event in events {
            self.ui.handle_event(event);
        }
    }

    fn start_interactive(&mut self) {
        self.runner.stop();
        let config = match self
            .ui
            .read_interactive_config(self.renderer.max_texture_size())
        {
            Ok(config) => config,
            Err(error) => {
                self.ui
                    .show_setup(Some(&format!("Invalid interactive configuration: {error}")));
                return;
            }
        };
        self.renderer.begin_texture_set(config.texture_size);
        self.interactive = Some(InteractiveSession {
            config,
            prepared_textures: 0,
            configured_scene: None,
            last_raf: None,
            smoothed_fps: None,
        });
        self.ui.begin_interactive();
    }

    fn tick_interactive(&mut self, now: f64) {
        let requested = match self
            .ui
            .read_interactive_config(self.renderer.max_texture_size())
        {
            Ok(config) => {
                self.ui.set_interactive_error(None);
                config
            }
            Err(error) => {
                self.ui.set_interactive_error(Some(&error));
                return;
            }
        };
        let session = self.interactive.as_mut().unwrap();

        if requested.texture_size != session.config.texture_size
            || requested.texture_count != session.config.texture_count
        {
            self.renderer.begin_texture_set(requested.texture_size);
            session.prepared_textures = 0;
            session.configured_scene = None;
            session.last_raf = None;
            session.smoothed_fps = None;
        }
        session.config = requested;

        if session.prepared_textures < session.config.texture_count {
            match self.renderer.prepare_next_texture() {
                Ok(()) => {
                    session.prepared_textures = self.renderer.prepared_texture_count();
                    self.ui.set_interactive_status(&format!(
                        "Preparing square {}×{} textures · {}/{}",
                        session.config.texture_size,
                        session.config.texture_size,
                        session.prepared_textures,
                        session.config.texture_count
                    ));
                }
                Err(error) => self.ui.set_interactive_error(Some(&error)),
            }
            return;
        }

        let scene_config = (session.config.image_count, session.config.draw_size);
        if session.configured_scene != Some(scene_config) {
            if let Err(error) = self
                .renderer
                .configure_scene(session.config.image_count, session.config.draw_size)
            {
                self.ui.set_interactive_error(Some(&error));
                return;
            }
            session.configured_scene = Some(scene_config);
            session.last_raf = None;
            session.smoothed_fps = None;
        }

        if let Err(error) = self.renderer.render_once(session.config.mode, now) {
            self.ui.set_interactive_error(Some(&error));
            return;
        }
        if let Some(last) = session.last_raf {
            let elapsed = now - last;
            if elapsed.is_finite() && elapsed > 0.0 && elapsed < 500.0 {
                let instantaneous = 1000.0 / elapsed;
                session.smoothed_fps = Some(match session.smoothed_fps {
                    Some(fps) => fps * 0.9 + instantaneous * 0.1,
                    None => instantaneous,
                });
            }
        }
        session.last_raf = Some(now);
        let fps = session
            .smoothed_fps
            .map_or("warming…".to_string(), |fps| format!("{fps:.1} FPS"));
        self.ui.set_interactive_status(&format!(
            "{} · {} rects · {fps}",
            session.config.mode.label(),
            session.config.image_count,
        ));
    }

    fn stop(&mut self) {
        self.runner.stop();
        self.interactive = None;
        self.renderer.delete_textures();
        self.ui.show_setup(None);
    }
}

#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let _ = console_log::init_with_level(log::Level::Info);

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("missing window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("missing document"))?;
    let canvas: HtmlCanvasElement = document
        .get_element_by_id("benchmark-canvas")
        .ok_or_else(|| JsValue::from_str("missing #benchmark-canvas"))?
        .dyn_into()?;
    let renderer = BenchRenderer::new(&canvas).map_err(|error| JsValue::from_str(&error))?;
    let ui = Ui::new(&document).map_err(|error| JsValue::from_str(&error))?;
    ui.set_device_info(renderer.max_texture_size(), renderer.viewport_dimensions());

    let app = Rc::new(RefCell::new(App {
        renderer,
        runner: BenchRunner::new(),
        ui,
        interactive: None,
    }));

    {
        let app = app.clone();
        let button = app.borrow().ui.run_button();
        let callback = Closure::<dyn FnMut()>::new(move || app.borrow_mut().start());
        button.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
        callback.forget();
    }

    {
        let app = app.clone();
        let button = app.borrow().ui.interactive_button();
        let callback = Closure::<dyn FnMut()>::new(move || app.borrow_mut().start_interactive());
        button.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
        callback.forget();
    }

    {
        let app = app.clone();
        let button = app.borrow().ui.stop_button();
        let callback = Closure::<dyn FnMut()>::new(move || app.borrow_mut().stop());
        button.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
        callback.forget();
    }

    {
        let app = app.clone();
        let button = app.borrow().ui.interactive_back_button();
        let callback = Closure::<dyn FnMut()>::new(move || app.borrow_mut().stop());
        button.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
        callback.forget();
    }

    {
        let app = app.clone();
        let button = app.borrow().ui.restart_button();
        let callback = Closure::<dyn FnMut()>::new(move || app.borrow().ui.show_setup(None));
        button.add_event_listener_with_callback("click", callback.as_ref().unchecked_ref())?;
        callback.forget();
    }

    start_animation_loop(app)?;
    Ok(())
}

fn start_animation_loop(app: Rc<RefCell<App>>) -> Result<(), JsValue> {
    let callback_slot = Rc::new(RefCell::new(None::<Closure<dyn FnMut(f64)>>));
    let callback_slot_inner = callback_slot.clone();
    let callback = Closure::<dyn FnMut(f64)>::new(move |now| {
        app.borrow_mut().tick(now);
        if let (Some(window), Some(callback)) =
            (web_sys::window(), callback_slot_inner.borrow().as_ref())
        {
            let _ = window.request_animation_frame(callback.as_ref().unchecked_ref());
        }
    });
    *callback_slot.borrow_mut() = Some(callback);
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("missing window"))?;
    window.request_animation_frame(
        callback_slot
            .borrow()
            .as_ref()
            .unwrap()
            .as_ref()
            .unchecked_ref(),
    )?;
    Ok(())
}

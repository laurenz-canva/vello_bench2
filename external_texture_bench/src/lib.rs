mod benchmark;
mod renderer;
mod ui;

use std::cell::RefCell;
use std::rc::Rc;

use benchmark::BenchRunner;
use renderer::BenchRenderer;
use ui::Ui;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

struct App {
    renderer: BenchRenderer,
    runner: BenchRunner,
    ui: Ui,
}

impl App {
    fn tick(&mut self, now: f64) {
        let hidden = web_sys::window()
            .and_then(|window| window.document())
            .is_some_and(|document| document.hidden());
        if hidden {
            self.runner.invalidate_pending_timing();
            return;
        }

        let events = self.runner.tick(&mut self.renderer, now);
        for event in events {
            self.ui.handle_event(event);
        }
    }

    fn start(&mut self) {
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

    fn stop(&mut self) {
        self.runner.stop();
        self.renderer.delete_textures();
        self.ui.show_setup(Some("Benchmark stopped"));
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
    ui.set_device_info(renderer.max_texture_size());

    let app = Rc::new(RefCell::new(App {
        renderer,
        runner: BenchRunner::new(),
        ui,
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
        let button = app.borrow().ui.stop_button();
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

//! Entry points for browsers, called from `web/index.html` and `web/worker.js`.

use std::{cell::RefCell, sync::Arc};

use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
use winit::window::Window;

use crate::{dimension::Dimension, gui, seed::parse_seed};

/// Starts the viewer in the canvas with id `canvas_id`. The page's query can pick the world:
/// `?seed=hello&dimension=nether`.
#[wasm_bindgen]
pub fn start(canvas_id: &str) -> Result<(), JsValue> {
    setup_logging();
    let window = web_sys::window().ok_or("no window")?;
    let canvas = window
        .document()
        .and_then(|document| document.get_element_by_id(canvas_id))
        .ok_or_else(|| format!("no element with id {canvas_id}"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;

    let query = web_sys::UrlSearchParams::new_with_str(&window.location().search()?)?;
    let seed = query.get("seed").and_then(|seed| parse_seed(&seed)).unwrap_or(0);
    let dimension = query
        .get("dimension")
        .and_then(|dimension| Dimension::from_name(&dimension))
        .unwrap_or(Dimension::Overworld);

    gui::start(canvas, seed, dimension).map_err(|err| JsValue::from_str(&format!("{err:#}")))
}

/// Generates a tile in a web worker, see `gui::executor::worker_run`.
#[wasm_bindgen]
pub fn worker_run(request: js_sys::Array) -> Result<js_sys::Array, JsValue> {
    setup_logging();
    gui::executor::worker_run(&request)
}

/// Answers a finder job in a web worker, see `gui::finder_pool::worker_run`.
#[wasm_bindgen]
pub fn finder_run(message: &str) -> Result<String, JsValue> {
    setup_logging();
    gui::finder_pool::worker_run(message)
}

#[wasm_bindgen]
extern "C" {
    /// Opens or closes the on screen keyboard, see `web/index.html`.
    #[wasm_bindgen(js_name = cuboxKeyboard)]
    fn cubox_keyboard(open: bool);
}

thread_local! {
    static WANTS_KEYBOARD: RefCell<bool> = const { RefCell::new(false) };
    static TYPED: RefCell<Vec<egui::Event>> = const { RefCell::new(Vec::new()) };
    /// to draw again when something was typed
    static WINDOW: RefCell<Option<Arc<Window>>> = const { RefCell::new(None) };
}

pub(crate) fn set_window(window: Arc<Window>) {
    WINDOW.set(Some(window));
}

pub(crate) fn set_keyboard(open: bool) {
    if WANTS_KEYBOARD.replace(open) != open {
        cubox_keyboard(open);
    }
}

pub(crate) fn take_typed_events() -> Vec<egui::Event> {
    TYPED.take()
}

/// Something typed on the on screen keyboard: text, or `Backspace` or `Enter` with `text` null.
#[wasm_bindgen]
pub fn typed(text: Option<String>, key: Option<String>) {
    TYPED.with_borrow_mut(|typed| match (text, key.as_deref()) {
        (Some(text), _) => typed.push(egui::Event::Text(text)),
        (None, Some(name)) => {
            let Some(key) = egui::Key::from_name(name) else { return };
            for pressed in [true, false] {
                typed.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: Default::default(),
                });
            }
        }
        (None, None) => {}
    });
    WINDOW.with_borrow(|window| {
        if let Some(window) = window {
            window.request_redraw();
        }
    });
}

fn setup_logging() {
    console_error_panic_hook::set_once();
    // only the first call sets it up, the others fail harmlessly
    let _ = console_log::init_with_level(log::Level::Info);
}

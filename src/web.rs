//! Entry points for browsers, called from `web/index.html` and `web/worker.js`.

use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};

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

fn setup_logging() {
    console_error_panic_hook::set_once();
    // only the first call sets it up, the others fail harmlessly
    let _ = console_log::init_with_level(log::Level::Info);
}

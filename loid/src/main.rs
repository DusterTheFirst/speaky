#![forbid(unsafe_code)]

mod app;

use app::Loid;
use common::color_eyre;

#[cfg(all(not(target_arch = "wasm32"), feature = "snmalloc"))]
#[global_allocator]
static ALLOC: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() -> color_eyre::Result<()> {
    // TODO: Better error handling

    eframe::run_native(Loid::initialize()?, eframe::NativeOptions::default())
}

// ----------------------------------------------------------------------------
// When compiling for web:

#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

/// This is the entry-point for all the web-assembly.
/// This is called once from the HTML.
/// It loads the app, installs some callbacks, then returns.
/// You can add more callbacks like this if you want to call in to your code.
#[cfg(target_arch = "wasm32")]
pub fn main() -> Result<(), eframe::wasm_bindgen::JsValue> {
    console_error_panic_hook::set_once();

    // TODO: Better error handling
    eframe::start_web(
        "egui_canvas",
        Loid::initialize().map_err(|err| err.to_string())?,
    )
}

#![forbid(unsafe_code)]

mod app;

use app::Application;
use common::{
    color_eyre::{self, eyre::Context},
    install_tracing,
    tracing::*,
};

fn init() -> color_eyre::Result<Application> {
    #[cfg(not(target_arch = "wasm32"))]
    color_eyre::install().wrap_err("failed to install color_eyre")?;

    install_tracing().wrap_err("failed to install tracing_subscriber")?;

    info!("Starting Application");

    Application::initialize()
}

#[cfg(all(not(target_arch = "wasm32"), feature = "snmalloc"))]
#[global_allocator]
static ALLOC: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() -> color_eyre::Result<()> {
    eframe::run_native(Box::new(init()?), eframe::NativeOptions::default())
}

// ----------------------------------------------------------------------------
// When compiling for web:

/// This is the entry-point for all the web-assembly.
/// This is called once from the HTML.
/// It loads the app, installs some callbacks, then returns.
/// You can add more callbacks like this if you want to call in to your code.
#[cfg(target_arch = "wasm32")]
fn main() {
    std::panic::set_hook(Box::new(|panic| {
        // FIXME: DO NOT PANIC IN PANIC HANDLER
        let window = web_sys::window().expect("no global `window` exists");
        let document = window.document().expect("should have a document on window");

        let canvas = document
            .get_element_by_id("egui_canvas")
            .expect("egui_canvas does not exist");
        canvas.remove();

        let text_agent = document
            .get_element_by_id("egui_text_agent")
            .expect("egui_text_agent does not exist");
        text_agent.remove();

        web_sys::console::error_1(&format!("{panic}").into())
    }));

    let app = match init() {
        Ok(app) => app,
        Err(err) => {
            error!("Encountered error in application initialization");

            // TODO: use console features to make this more integrated
            web_sys::console::error_1(
                &strip_ansi_escapes::strip_str(format!("Error: {err:?}")).into(),
            );

            return;
        }
    };

    match eframe::start_web("egui_canvas", Box::new(app)) {
        Ok(()) => {
            info!("eframe successfully started");
        }
        Err(error) => {
            error!(?error, "eframe encountered an error");
        }
    }

    panic!()

    // TODO: unregister all event listeners on panic
}

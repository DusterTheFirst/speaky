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
    // Use console error panic hook until we need to upgrade to our custom one
    console_error_panic_hook::set_once();

    let hash = web_sys::window()
        .expect("no global `window` exists")
        .location()
        .hash()
        .expect("unable to get location hash");

    if !hash.is_empty() {
        let hash = js_sys::decode_uri_component(&hash[1..]).expect("amongus");
        web_sys::console::error_1(&hash.into());
        // TODO: more cool message

        return;
    }

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

    // Upgrade to bigger panic handler to detach egui
    // TODO: raise issue in egui_web about this?
    // TODO: unregister all event listeners on panic
    // std::panic::set_hook(Box::new(|panic| {
    //     // FIXME: DO NOT PANIC IN PANIC HANDLER
    //     let window = web_sys::window().expect("no global `window` exists");

    //     let location = window.location();

    //     // TODO:
    //     // location
    //     //     .set_hash(
    //     //         &js_sys::encode_uri_component(&format!("{panic}"))
    //     //             .as_string()
    //     //             .expect("JsString is a string"),
    //     //     )
    //     //     .expect("unable to get location hash");

    //     // location.reload().expect("unable to reload");

    //     // FIXME: prevent normal panic hook from running
    // }));


    match eframe::start_web("egui_canvas", Box::new(app)) {
        Ok(()) => {
            info!("eframe successfully started");
        }
        Err(error) => {
            error!(?error, "eframe encountered an error");
        }
    }
}

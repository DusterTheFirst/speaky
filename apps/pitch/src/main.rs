#![deny(clippy::panic_in_result_fn)]
#![forbid(unsafe_code)]
// #![warn(clippy::unwrap_used, clippy::expect_used)]

use color_eyre::eyre::Context;
use eframe::{NativeOptions, APP_KEY};
use ritelinked::LinkedHashSet;
use tracing::info;
use util::install_tracing;

use crate::app::Application;

mod analysis;
mod app;
mod decode;
mod key;
mod midi;
mod piano_roll;
mod ui_error;

pub const NAME: &str = "Pitch";

pub fn main() -> color_eyre::Result<()> {
    color_eyre::install().wrap_err("failed to install color_eyre")?;

    install_tracing().wrap_err("failed to install tracing_subscriber")?;

    info!("Starting Application");

    eframe::run_native(
        NAME,
        NativeOptions::default(),
        Box::new(|cc| {
            let recently_opened_files = if let Some(storage) = cc.storage {
                eframe::get_value(storage, APP_KEY).unwrap_or_default()
            } else {
                LinkedHashSet::new()
            };

            Box::new(Application::new(recently_opened_files))
        }),
    )
}

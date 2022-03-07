use color_eyre::eyre::Context;
use eframe::NativeOptions;
use tracing::info;
use util::install_tracing;

use crate::app::Application;

mod app;
mod key;
mod piano_roll;

pub fn main() -> color_eyre::Result<()> {
    color_eyre::install().wrap_err("failed to install color_eyre")?;

    install_tracing().wrap_err("failed to install tracing_subscriber")?;

    info!("Starting Application");

    eframe::run_native(
        Box::new(Application::default()),
        NativeOptions {
            drag_and_drop_support: true,
            ..Default::default()
        },
    )
}

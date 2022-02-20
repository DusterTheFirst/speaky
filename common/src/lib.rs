#![deny(clippy::unwrap_used, clippy::expect_used)]

use color_eyre::eyre::Context;

pub use color_eyre;
pub use rodio;

pub mod audio;
pub mod spectrum;

#[cfg(not(target_arch = "wasm32"))]
pub mod tts;

pub fn install_tracing() -> color_eyre::Result<()> {
    use tracing_error::ErrorLayer;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};

    let fmt_layer = fmt::layer().pretty();
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .wrap_err("unable to create env filter")?;

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .with(ErrorLayer::default())
        .init();

    Ok(())
}

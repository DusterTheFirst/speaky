#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![warn(missing_copy_implementations, missing_debug_implementations)]

use color_eyre::eyre::Context;

pub fn install_tracing() -> color_eyre::Result<()> {
    use tracing_error::ErrorLayer;
    use tracing_subscriber::{prelude::*, EnvFilter};

    #[cfg(target_arch = "wasm32")]
    let fmt_layer = tracing_wasm::WASMLayer::default();

    #[cfg(not(target_arch = "wasm32"))]
    let fmt_layer = tracing_subscriber::fmt::layer().pretty();

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

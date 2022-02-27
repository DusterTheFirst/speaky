#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![warn(missing_copy_implementations, missing_debug_implementations)]

#[cfg(feature = "io")]
pub mod input;

#[cfg(feature = "io")]
pub mod output;

pub mod waveform;

#[cfg(feature = "cpal")]
pub use cpal::Sample;
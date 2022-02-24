#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![warn(missing_copy_implementations, missing_debug_implementations)]

#[cfg(feature = "io")]
pub mod input;
pub mod waveform;

#[cfg(feature = "cpal")]
pub extern crate cpal;
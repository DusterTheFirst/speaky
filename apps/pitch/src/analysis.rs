use std::collections::BTreeMap;

use audio::waveform::Waveform;
use eframe::epaint::{Color32, ColorImage};
use spectrum::WaveformSpectrum;

use crate::{
    key::PianoKey,
    piano_roll::{KeyDuration, KeyPress, KeyPresses},
};

#[derive(Debug, Clone, Copy)]
pub struct AnalysisOptions {
    pub fft_width: u8,

    pub window_fraction: f32,
    pub step_fraction: f32,

    pub threshold: f32,
}

pub fn analyze(
    waveform: &Waveform,
    options: AnalysisOptions,
    progress_callback: &dyn Fn(f32),
) -> (BTreeMap<PianoKey, KeyPresses>, ColorImage) {
    let fft_width = 1 << options.fft_width;
    let window_width = (fft_width as f32 * options.window_fraction).ceil() as usize;
    let step = (window_width as f32 * options.step_fraction).ceil() as usize;

    let windows = (0..waveform.len() - window_width)
        .step_by(step)
        .map(|start| start..start + window_width);
    let window_count = dbg!(windows.len());

    let seconds_per_window = window_width as f64 / waveform.sample_rate() as f64;

    let mut image = ColorImage::new([window_count, fft_width / 2], Color32::BLACK);
    let mut keys = BTreeMap::<PianoKey, KeyPresses>::new();

    for (i, window) in windows.enumerate() {
        progress_callback(i as f32 / image.width() as f32);

        let waveform = waveform.slice(window);
        let spectrum = waveform.spectrum(spectrum::Window::Hann, fft_width);

        let width = image.width();
        // let mut max = None;
        for (pixel, (bucket, amplitude)) in image.pixels[i..]
            .iter_mut()
            .step_by(width)
            .zip(spectrum.amplitudes_real().enumerate())
        {
            let amp_u8 = (u8::MAX as f32 * amplitude).round() as u8;
            *pixel = Color32::from_rgb(amp_u8, amp_u8, amp_u8);

            // let max = max.get_or_insert((bucket, amplitude));
            // if amplitude > max.1 {
            //     *max = (bucket, amplitude)
            // }

            if amplitude < options.threshold {
                continue;
            }

            let frequency = spectrum.freq_from_bucket(bucket) as f32;
            let key = PianoKey::from_concert_pitch(frequency);

            if let Some(key) = key {
                keys.entry(key).or_default().add(KeyPress::new(
                    (i as f64 * seconds_per_window * 1000.0).round() as u64,
                    KeyDuration::from_secs_f64(seconds_per_window),
                    amplitude,
                ));
            }
        }
    }

    (keys, image)
}

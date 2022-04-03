use std::collections::BTreeMap;

use audio::waveform::Waveform;
use eframe::epaint::{Color32, ColorImage};
use spectrum::WaveformSpectrum;

use crate::{
    key::PianoKey,
    piano_roll::{KeyDuration, KeyPress, KeyPresses},
};

pub fn analyze(
    waveform: &Waveform,
    threshold: f32,
    progress_callback: &dyn Fn(f32),
) -> (BTreeMap<PianoKey, KeyPresses>, ColorImage) {
    const FFT_WIDTH: usize = 8192;
    const WINDOW_WIDTH: usize = FFT_WIDTH / 2;

    let windows = (0..waveform.len() - WINDOW_WIDTH)
        .step_by(WINDOW_WIDTH)
        .map(|start| start..start + WINDOW_WIDTH);
    let window_count = dbg!(windows.len());

    let seconds_per_window = WINDOW_WIDTH as f64 / waveform.sample_rate() as f64;

    let mut image = ColorImage::new([window_count, FFT_WIDTH / 2], Color32::BLACK);
    let mut keys = BTreeMap::<PianoKey, KeyPresses>::new();

    for (i, window) in windows.enumerate() {
        progress_callback(i as f32 / image.width() as f32);

        let waveform = waveform.slice(window);
        let spectrum = waveform.spectrum(spectrum::Window::Hann, FFT_WIDTH);

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

            if amplitude < threshold {
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

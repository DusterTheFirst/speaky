use std::{collections::BTreeMap, time::Duration};

use audio::waveform::Waveform;
use eframe::epaint::{Color32, ColorImage};
use spectrum::WaveformSpectrum;

use crate::key::PianoKey;

#[derive(Debug, Clone, Copy)]
pub struct AnalysisOptions {
    pub fft_size: u8,

    pub window_fraction: f32,
    pub step_fraction: f32,

    pub threshold: f32,
}

impl AnalysisOptions {
    pub fn fft_width(&self) -> usize {
        1 << self.fft_size
    }
    pub fn window_width(&self) -> usize {
        (self.fft_width() as f32 * self.window_fraction).ceil() as usize
    }
    pub fn step(&self) -> usize {
        (self.window_width() as f32 * self.step_fraction).ceil() as usize
    }
}

pub fn analyze(
    waveform: &Waveform,
    options: AnalysisOptions,
    progress_callback: &dyn Fn(f32),
) -> (BTreeMap<PianoKey, KeyPresses>, ColorImage) {
    let fft_width = options.fft_width();
    let window_width = options.window_width();
    let step = options.step();

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
            let color = colorous::VIRIDIS.eval_continuous(amplitude as f64);
            *pixel = Color32::from_rgb(color.r, color.g, color.b);

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

// FIXME: better data representation?
// The start of the keypress in milliseconds
pub type KeyStart = u128;
// The duration of the keypress
pub type KeyDuration = Duration;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct KeyPress {
    start: KeyStart,
    info: KeyPressInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
struct KeyPressInfo {
    duration: KeyDuration,
    intensity: f32,
}

impl KeyPress {
    pub fn new(
        start: impl Into<KeyStart>,
        duration: KeyDuration,
        intensity: impl Into<f32>,
    ) -> Self {
        Self {
            start: start.into(),
            info: KeyPressInfo {
                duration,
                intensity: intensity.into(),
            },
        }
    }

    pub fn start(&self) -> u128 {
        self.start
    }

    pub fn start_secs(&self) -> f32 {
        self.start as f32 / 1000.0
    }

    pub fn duration(&self) -> Duration {
        self.info.duration
    }

    pub fn duration_secs(&self) -> f32 {
        self.info.duration.as_secs_f32()
    }

    pub fn end_secs(&self) -> f32 {
        self.start_secs() + self.duration_secs()
    }

    pub fn intensity(&self) -> f32 {
        self.info.intensity
    }
}

#[derive(Debug, Default, Clone)]
pub struct KeyPresses {
    key_list: BTreeMap<KeyStart, KeyPressInfo>,
}

impl FromIterator<KeyPress> for KeyPresses {
    fn from_iter<T: IntoIterator<Item = KeyPress>>(iter: T) -> Self {
        let mut presses = Self::new();
        presses.extend(iter);
        presses
    }
}

impl<const N: usize> From<[KeyPress; N]> for KeyPresses {
    fn from(array: [KeyPress; N]) -> Self {
        let mut presses = Self::new();
        presses.extend(array);
        presses
    }
}

impl Extend<KeyPress> for KeyPresses {
    fn extend<T: IntoIterator<Item = KeyPress>>(&mut self, iter: T) {
        for keypress in iter.into_iter() {
            self.add(keypress);
        }
    }
}

impl KeyPresses {
    pub fn new() -> Self {
        Self {
            key_list: BTreeMap::new(),
        }
    }

    pub fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = KeyPress> + DoubleEndedIterator<Item = KeyPress> + '_ {
        self.key_list
            .iter()
            .map(|(&start, &info)| KeyPress { start, info })
    }

    pub fn len(&self) -> usize {
        self.key_list.len()
    }

    pub fn first(&self) -> Option<KeyPress> {
        self.iter().next()
    }

    pub fn last(&self) -> Option<KeyPress> {
        self.iter().next_back()
    }

    // FIXME: what do about intensity
    // FIXME: do at analysis time?
    pub fn add(&mut self, mut keypress: KeyPress) {
        // Join with the note before this
        if let Some((
            previous_key_start,
            KeyPressInfo {
                duration: previous_key_duration,
                ..
            },
        )) = self.key_list.range_mut(..keypress.start).next_back()
        {
            // Check if the end of the previous keypress overlaps with the start of this keypress
            if *previous_key_start + previous_key_duration.as_millis() == keypress.start {
                // Extend the previous key's duration
                *previous_key_duration += keypress.duration();

                return;
            }
        }

        // Join with the note after this
        if let Some((
            &next_key_start,
            &KeyPressInfo {
                duration: next_key_duration,
                ..
            },
        )) = self.key_list.range(keypress.start..).next()
        {
            // Check if the end of this keypress overlaps with the start of the next keypress
            if keypress.start + keypress.duration().as_millis() == next_key_start {
                // Extend this key's duration
                keypress.info.duration += next_key_duration;

                // Remove the note after this
                self.key_list.remove(&next_key_start);
            }
        }

        self.key_list.insert(keypress.start, keypress.info);
    }

    // FIXME: Does not verify duration
    pub fn remove(&mut self, keypress: &KeyPress) {
        self.key_list.remove(&keypress.start);
    }
}

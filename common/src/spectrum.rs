use std::{
    borrow::Cow,
    cmp::Ordering,
    f32::consts,
    fmt::{self, Display},
    iter,
    ops::Range,
};

use num_complex::Complex;

use crate::spectrum::fft::cfft;

mod fft {
    use num_complex::Complex;

    macro_rules! variable_width_fft {
        (
            use $algor:path;

            match $samples:ident in [
                $($num:literal),+
            ]
        ) => {
            match $samples.len() {
                $(
                    $num => paste::paste! {{
                        [<$algor _ $num>](TryFrom::<&mut [Complex<f32>]>::try_from($samples).expect(concat!("spectrum.len() != ", $num))) as &mut [Complex<f32>]
                    }},
                )+
                _ => unimplemented!("unsupported width"),
            }
        };
    }

    pub fn cfft(samples: &mut [Complex<f32>]) {
        use microfft::complex::*;

        variable_width_fft! {
            use cfft;

            match samples in [
                2, 4, 8, 16, 32, 64,
                128, 256, 512, 1024,
                2048, 4096, 8192, 16384
            ]
        };
    }
}

// pub fn pitch_change(samples: &[f32])

#[deprecated]
pub fn reconstruct_samples(
    full_spectrum: &[Complex<f32>],
    work_buffer: &mut Vec<Complex<f32>>,
    samples: &mut Vec<f32>,
    width: usize,
) {
    debug_assert_eq!(
        full_spectrum.len(),
        width,
        "full spectrum width does not match fft width"
    );

    work_buffer.clear();
    work_buffer.extend(
        full_spectrum
            .iter()
            .map(|complex| Complex::new(complex.im, complex.re)),
    );
    samples.shrink_to_fit();

    cfft(work_buffer);

    samples.clear();
    samples.extend(work_buffer.iter().map(|complex| complex.im / width as f32));
    samples.shrink_to_fit();
}

// TODO: signed shift?
#[deprecated]
pub fn shift_spectrum(
    buckets: usize,

    spectrum: &[Complex<f32>],
    shifted_spectrum: &mut Vec<Complex<f32>>,
) {
    shifted_spectrum.clear();

    // If the result would shift all components off, take a shortcut and just fill it with zeros
    if buckets >= spectrum.len() / 2 {
        shifted_spectrum.resize(spectrum.len(), Complex::new(0.0, 0.0));
        return;
    }

    let zero_iter = iter::repeat(Complex::new(0.0, 0.0)).take(buckets);
    let half_spectrum_length = spectrum.len() / 2 - buckets;

    let (second_half_skip, second_zero_skip) = if buckets == 0 { (1, 0) } else { (0, 1) };

    shifted_spectrum.extend(
        zero_iter
            .clone()
            .chain(spectrum.iter().copied().take(half_spectrum_length + 1))
            .chain(
                spectrum
                    .iter()
                    .map(Complex::conj)
                    .take(half_spectrum_length)
                    .skip(second_half_skip)
                    .rev(),
            )
            .chain(zero_iter.skip(second_zero_skip)),
    );
}

#[deprecated]
pub fn scale_spectrum(
    scale: f32,

    spectrum: &[Complex<f32>],
    scaled_spectrum: &mut Vec<Complex<f32>>,
) {
    scaled_spectrum.clear();
    scaled_spectrum.resize(spectrum.len(), Complex::new(0.0, 0.0));
    scaled_spectrum.shrink_to_fit();

    let width = spectrum.len();
    let half_width = width / 2 + 1;

    // Copy DC offset
    scaled_spectrum[0].re = spectrum[0].re;

    // TODO: do something about the nyquist frequency (imaginary component of DC)

    // Iterate over all real frequencies, saving them into the new spectrum
    for (bucket, component) in spectrum
        .iter()
        .take(half_width)
        .copied()
        .enumerate()
        .skip(1)
    {
        // TODO: non-integer scaling
        let bucket = (bucket as f32 * scale).round() as usize;

        if bucket > half_width {
            break;
        }

        // TODO: way to let the compiler know bounds checks are not needed?
        scaled_spectrum[bucket] = component;
    }

    // Split the spectrum at one over half since 1-nyquist is shared between the two
    let (original, mirror) = scaled_spectrum.split_at_mut(half_width);

    // Skip the DC offset which is only present in the left hand side
    let original = original.iter().skip(1);

    // Reverse the order that we iterate through the mirror
    let mirror = mirror.iter_mut().rev();

    // Mirror changes to other half of spectrum
    for (original, mirror) in original.zip(mirror) {
        // let gamma = scale * original.arg() * TODO:
        *mirror = original.conj();
    }
}

/// Helper function to wrap a phase between -[π] and [π]
///
/// [π]: std::f32::consts::PI
fn wrap_phase(phase: f32) -> f32 {
    if phase >= 0.0 {
        ((phase + consts::PI) % consts::TAU) - consts::PI
    } else {
        ((phase - consts::PI) % -consts::TAU) + consts::PI
    }
}

pub struct Spectrum<'waveform> {
    width: usize,
    buckets: Box<[Complex<f32>]>,
    waveform: &'waveform Waveform<'waveform>,
}

impl<'w> Spectrum<'w> {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn buckets(&self) -> &[Complex<f32>] {
        &self.buckets
    }

    pub fn amplitudes(&self) -> impl Iterator<Item = f32> + '_ {
        self.buckets.iter().map(|complex| complex.norm())
    }

    pub fn phases(&self) -> impl Iterator<Item = f32> + '_ {
        self.buckets
            .iter()
            .map(|complex| complex.arg() / self.width as f32)
    }

    pub fn amplitudes_real(&self) -> impl Iterator<Item = f32> + '_ {
        self.amplitudes().take(self.width / 2 + 1)
    }

    pub fn phases_real(&self) -> impl Iterator<Item = f32> + '_ {
        self.phases().take(self.width / 2 + 1)
    }

    // TODO: rename?
    pub fn main_frequency(&self) -> Option<(usize, f32)> {
        self.amplitudes_real()
            .enumerate()
            .max_by(|&(_, amp_1), &(_, amp_2)| {
                amp_1.partial_cmp(&amp_2).unwrap_or_else(|| {
                    // Choose the non-nan value
                    match (amp_1.is_nan(), amp_2.is_nan()) {
                        (true, true) => panic!("encountered two NaN values"),
                        (false, true) => Ordering::Greater,
                        (true, false) => Ordering::Less,
                        (false, false) => unreachable!(),
                    }
                })
            })
    }

    pub fn freq_resolution(&self) -> f64 {
        (1.0 / self.width as f64) * self.waveform.sample_rate as f64
    }

    pub fn freq_from_bucket(&self, bucket: usize) -> f64 {
        if bucket > self.width / 2 {
            -((self.width - bucket) as f64 * self.freq_resolution())
        } else {
            bucket as f64 * self.freq_resolution()
        }
    }

    pub fn bucket_from_freq(&self, freq: f64) -> usize {
        ((freq * self.width as f64) / self.waveform.sample_rate as f64).round() as usize
    }

    // TODO: signed shift?
    pub fn shift(&self, shift: usize) -> Spectrum<'w> {
        let half_spectrum = self.width / 2;

        Spectrum {
            width: self.width,
            waveform: self.waveform,
            buckets: iter::repeat(Complex::new(0.0, 0.0))
                .take(shift)
                .chain(self.buckets[..(half_spectrum - shift)].iter().copied())
                .chain(self.buckets[(half_spectrum + shift)..].iter().copied())
                .chain(iter::repeat(Complex::new(0.0, 0.0)).take(shift))
                .collect(),
        }
    }
}

pub struct Waveform<'s> {
    samples: Cow<'s, [f32]>,
    sample_rate: u32,
}

impl Waveform<'_> {
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples: Cow::Owned(samples),
            sample_rate,
        }
    }

    pub fn slice(&self, range: Range<usize>) -> Waveform {
        Waveform {
            sample_rate: self.sample_rate,
            samples: Cow::Borrowed(&self.samples[range]),
        }
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn time_from_sample(&self, sample: usize) -> f32 {
        sample as f32 / self.sample_rate as f32
    }

    pub fn time_domain(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.samples
            .iter()
            .enumerate()
            .map(|(sample, x)| (self.time_from_sample(sample), *x))
    }
}

impl Waveform<'_> {
    // TODO: see if rfft would be worth using unsafe for over cfft
    pub fn spectrum(&self, window: Window, window_width: usize) -> Spectrum {
        debug_assert!(
            self.len() >= window_width,
            "not enough samples provided. expected at least {window_width}, got {}",
            self.len()
        );
        assert!(
            self.len().is_power_of_two(),
            "waveform length must be a power of two"
        );

        let window = window.into_iter(window_width);

        // Copy samples into the spectrum, filling any extra space with zeros
        let mut buckets = self
            .samples()
            .iter()
            .copied()
            .zip(window.chain(iter::repeat(0.0)))
            .map(|(sample, scale)| Complex::new(sample * scale, 0.0))
            .take(self.len())
            .collect::<Box<_>>();

        // Perform the FFT based on the calculated width
        cfft(&mut buckets);

        Spectrum {
            buckets,
            width: self.len(),
            waveform: self,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    #[doc(alias = "Triangular")]
    Bartlett,
    Hamming,
    /// Good default choice
    Hann,
    Rectangular,
}

impl Display for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Window {
    pub const ALL: [Window; 4] = [Self::Bartlett, Self::Hamming, Self::Hann, Self::Rectangular];

    pub fn into_iter(self, width: usize) -> WindowIter {
        WindowIter {
            range: 0..width,
            width,
            window: self,
        }
    }
}

pub struct WindowIter {
    range: Range<usize>,
    width: usize,
    window: Window,
}

impl Iterator for WindowIter {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(n) = self.range.next() {
            let n = n as f32;
            let width = self.width as f32;

            Some(match self.window {
                Window::Rectangular => 1.0,
                Window::Bartlett => 1.0 - f32::abs((n - width / 2.0) / (width / 2.0)),
                Window::Hann => 0.5 * (1.0 - f32::cos((consts::TAU * n) / width)),
                Window::Hamming => {
                    (25.0 / 46.0) - ((21.0 / 46.0) * f32::cos((consts::TAU * n) / width))
                }
            })
        } else {
            None
        }
    }
}

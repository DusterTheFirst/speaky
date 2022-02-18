use std::{iter, ops::Range};

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

pub struct Spectrum<'analyzer, 'waveform> {
    width: usize,
    buckets: &'analyzer [Complex<f32>],
    waveform: &'waveform Waveform,
}

impl<'a, 'w> Spectrum<'a, 'w> {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn buckets(&self) -> &[Complex<f32>] {
        self.buckets
    }

    pub fn amplitudes(&self) -> impl Iterator<Item = f32> + '_ {
        self.buckets
            .iter()
            .map(|complex| complex.norm() / self.width as f32)
    }

    pub fn phases(&self) -> impl Iterator<Item = f32> + '_ {
        self.buckets.iter().map(|complex| complex.arg())
    }

    pub fn freq_from_bucket(&self, bucket: usize) -> f64 {
        bucket as f64 / self.width as f64 * self.waveform.sample_rate as f64
    }

    pub fn bucket_from_freq(&self, freq: f64) -> usize {
        ((freq * self.width as f64) / self.waveform.sample_rate as f64).round() as usize
    }

    // FIXME: probably wrong
    // TODO: signed shift?
    // pub fn shift(&mut self, range: Range<usize>, shift: usize) {
    //     let half_spectrum = self.buckets.len() / 2;
    //     let width_to_copy = half_spectrum - shift;

    //     // Shift real half right
    //     self.buckets.copy_within(0..width_to_copy, shift);

    //     // Shift imaginary half left
    //     self.buckets
    //         .copy_within((half_spectrum + width_to_copy).., half_spectrum);
    // }
}

pub struct Waveform {
    samples: Vec<f32>,
    sample_rate: u32,
}

impl Waveform {
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
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

#[derive(Debug, Default)]
pub struct WaveformAnalyzer {
    // Scratch buffer for dealing with complex numbers
    spectrum_buffer: Vec<Complex<f32>>,
}

impl WaveformAnalyzer {
    // TODO: see if rfft would be worth using unsafe for over cfft
    // TODO: windowing functions
    pub fn spectrum<'analyzer, 'waveform>(
        &'analyzer mut self,
        waveform: &'waveform Waveform,
        range: Range<usize>,
    ) -> Spectrum<'analyzer, 'waveform> {
        debug_assert!(
            range.start < range.end,
            "range end must be greater than range start"
        );

        let width = range.len();
        let width = width.next_power_of_two();

        // Resize the spectrum buffer to fit the
        self.spectrum_buffer.clear();

        // Copy samples into the spectrum, filling any extra space with zeros
        self.spectrum_buffer.extend(
            waveform.samples[range]
                .iter()
                .copied()
                .chain(iter::repeat(0.0))
                .map(|sample| Complex::new(sample, 0.0))
                .take(width),
        );

        // Perform the FFT based on the calculated width
        cfft(&mut self.spectrum_buffer);

        Spectrum {
            buckets: &self.spectrum_buffer,
            waveform,
            width,
        }
    }
}

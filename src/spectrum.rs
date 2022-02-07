use std::iter;

use num_complex::Complex;

macro_rules! variable_width_fft {
    (
        for $samples:ident match $width:ident in $type:ident [
            $($num:literal),+
        ]
    ) => {
        match $width {
            $(
                $num => paste::paste! {
                    [<$type _ $num>]($samples.try_into().expect(concat!("spectrum.len() != ", $num))) as &mut [Complex<f32>]
                },
            )+
            _ => unimplemented!("unsupported width"),
        }
    };
}

pub fn reconstruct_samples(full_spectrum: &[Complex<f32>], width: usize) -> Box<[f32]> {
    let spectrum_conjugate = full_spectrum
        .iter()
        .map(|complex| Complex::new(complex.im, complex.re));

    let mut spectrum = Vec::new();
    spectrum.extend(spectrum_conjugate);

    let spectrum = spectrum.as_mut_slice();

    use microfft::complex::*;

    let samples = variable_width_fft! {
        for spectrum match width in cfft [
            2, 4, 8, 16, 32, 64,
            128, 256, 512, 1024,
            2048, 4096, 8192, 16384
        ]
    };

    samples
        .iter()
        .map(|complex| complex.im / width as f32)
        .collect()
}

pub fn full_spectrum(half_spectrum: &[Complex<f32>]) -> Box<[Complex<f32>]> {
    // The real-valued coefficient at the Nyquist frequency
    // is packed into the imaginary part of the DC bin.
    let real_at_nyquist = half_spectrum[0].im;
    let dc = half_spectrum[0].re;

    let half_spectrum = half_spectrum.iter().skip(1).copied();

    iter::once(Complex::new(dc, 0.0))
        .chain(half_spectrum.clone())
        .chain(iter::once(Complex::new(real_at_nyquist, 0.0)))
        .chain(half_spectrum.rev().map(|complex| complex.conj()))
        .collect()
}

pub fn shift_spectrum(half_spectrum: &[Complex<f32>], scale: f32) -> Box<[Complex<f32>]> {
    let mut half_spectrum_rotate = vec![Complex::new(0.0, 0.0); half_spectrum.len()];

    // Copy DC offset
    half_spectrum_rotate[0] = half_spectrum[0];

    // Iterate over all frequencies, saving them into the new spectrum
    for (bucket, component) in half_spectrum.iter().copied().enumerate().skip(1) {
        let bucket = (bucket as f32 * scale).round() as usize;

        if let Some(new_component) = half_spectrum_rotate.get_mut(bucket) {
            *new_component = component;
        } else {
            break;
        }
    }

    // TODO: do something about the nyquist frequency (im comp of DC)

    half_spectrum_rotate.into_boxed_slice()
}

// TODO: stop allocating boxes and allow user to provide a scratch buffer
pub fn spectrum(samples: &[f32], start: usize, width: usize) -> Box<[Complex<f32>]> {
    assert!(
        samples.len() >= width,
        "fft requires at least {width} samples but was provided {}",
        samples.len()
    );
    assert!(
        start < samples.len() - width,
        "start position is too large. {start} >= {}",
        samples.len() - width
    );

    // Stack allocate the samples so the originals are not mutated by the fft
    let mut samples: Box<[f32]> = Box::from(&samples[start..(start + width)]);
    let samples = samples.as_mut();

    use microfft::real::*;

    let spectrum = variable_width_fft! {
        for samples match width in rfft [
            2, 4, 8, 16, 32, 64,
            128, 256, 512, 1024,
            2048, 4096, 8192, 16384
        ]
    };

    // Copy the reinterpreted buffer into a new box
    // Unsafe could be used to get around the clone but it may not be worth it
    Box::from(&spectrum[..])
}

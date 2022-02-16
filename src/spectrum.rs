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
                    [<$type _ $num>](TryFrom::<&mut [Complex<f32>]>::try_from($samples).expect(concat!("spectrum.len() != ", $num))) as &mut [Complex<f32>]
                },
            )+
            _ => unimplemented!("unsupported width"),
        }
    };
}

// pub fn pitch_change(samples: &[f32])

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

    {
        use microfft::complex::*;

        variable_width_fft! {
            for work_buffer match width in cfft [
                2, 4, 8, 16, 32, 64,
                128, 256, 512, 1024,
                2048, 4096, 8192, 16384
            ]
        };
    };

    samples.clear();
    samples.extend(work_buffer.iter().map(|complex| complex.im / width as f32));
    samples.shrink_to_fit();
}

// TODO: signed shift?
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

    shifted_spectrum.extend(
        zero_iter
            .clone()
            .chain(spectrum.iter().copied().take(half_spectrum_length + 1))
            .chain(
                spectrum
                    .iter()
                    .map(Complex::conj)
                    .take(half_spectrum_length)
                    .skip(1)
                    .rev(),
            )
            .chain(zero_iter),
    );
}

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

// TODO: stop allocating boxes and allow user to provide a scratch buffer
// TODO: see if rfft would be worth using unsafe for over cfft
pub fn spectrum<'sp>(
    start: usize,
    width: usize,

    samples: &[f32],
    spectrum: &'sp mut Vec<Complex<f32>>,
) -> &'sp mut [Complex<f32>] {
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

    spectrum.clear();
    spectrum.extend(
        samples[start..(start + width)]
            .iter()
            .copied()
            .map(|sample| Complex::new(sample, 0.0)),
    );
    spectrum.shrink_to_fit();

    use microfft::complex::*;

    variable_width_fft! {
        for spectrum match width in cfft [
            2, 4, 8, 16, 32, 64,
            128, 256, 512, 1024,
            2048, 4096, 8192, 16384
        ]
    }
}

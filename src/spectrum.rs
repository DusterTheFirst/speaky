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

pub fn shift_spectrum(
    spectrum: &[Complex<f32>],
    shifted_spectrum: &mut Vec<Complex<f32>>,
    buckets: usize,
) {
    shifted_spectrum.clear();

    let zero_iter = std::iter::repeat(Complex::new(0.0, 0.0)).take(buckets);
    let spectrum_iter = spectrum.iter().take(spectrum.len() / 2 - buckets).copied();

    shifted_spectrum.extend(
        zero_iter
            .clone()
            .chain(spectrum_iter.clone())
            .chain(spectrum_iter.clone().rev())
            .chain(zero_iter),
    );
}

pub fn scale_spectrum(
    spectrum: &[Complex<f32>],
    scaled_spectrum: &mut Vec<Complex<f32>>,
    scale: f32,
) {
    scaled_spectrum.clear();
    scaled_spectrum.resize(spectrum.len(), Complex::new(0.0, 0.0));
    scaled_spectrum.shrink_to_fit();

    let width = spectrum.len();
    let half_width = width / 2;

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
    let (original, mirror) = scaled_spectrum.split_at_mut(half_width + 1);

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
    samples: &[f32],
    start: usize,
    width: usize,
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

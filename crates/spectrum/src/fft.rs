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

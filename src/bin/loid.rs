use std::{iter, sync::Arc, thread};

use eframe::{
    egui::{
        plot::{Bar, BarChart, Legend, Plot, Points, VLine, Value, Values},
        Button, CentralPanel, CtxRef,
    },
    epi::{App, Frame},
    NativeOptions,
};
use microfft::{complex::cfft_16384, real::rfft_16384};
use num_complex::Complex;
use rodio::{buffer::SamplesBuffer, OutputStream, Sink, Source};
use speaky::{load_language, setup_tts, synthesize};

const N: usize = 16384;

fn main() {
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();

    let sink = Sink::try_new(&stream_handle).unwrap();

    let resources = load_language("en-US").unwrap();

    let mut engine = setup_tts(resources);

    let speech = synthesize(&mut engine, "Some Body Once");

    // let speech = SineWave::new(20.0).take_duration(Duration::from_secs_f32(1.0 / 10.0));

    let sample_rate = speech.sample_rate();
    let samples: Vec<f32> = speech.convert_samples().collect();

    let half_spectrum = {
        assert!(samples.len() >= N, "Too few samples");

        let mut fixed_sized_samples: Box<[f32; N]> = Box::new([0.0; N]);
        fixed_sized_samples.copy_from_slice(&samples[..N]);

        let fixed_sized_samples = Box::leak(fixed_sized_samples);

        let spectrum = rfft_16384(fixed_sized_samples);

        // This saves a large copy
        unsafe { Box::from_raw(spectrum) }
    };

    let full_spectrum = {
        // The real-valued coefficient at the Nyquist frequency
        // is packed into the imaginary part of the DC bin.
        let real_at_nyquist = half_spectrum[0].im;
        let dc = half_spectrum[0].re;

        let half_spectrum = half_spectrum.iter().skip(1).copied();

        iter::once(Complex::new(dc, 0.0))
            .chain(half_spectrum.clone())
            .chain(iter::once(Complex::new(real_at_nyquist, 0.0)))
            .chain(half_spectrum.map(|complex| complex.conj()).rev())
            .collect::<Vec<_>>()
    };

    let reconstructed_samples = {
        let spectrum_conjugate = full_spectrum
            .iter()
            .map(|complex| Complex::new(complex.im, complex.re));

        let mut fixed_sized_spectrum: Box<[Complex<f32>; N]> =
            Box::new([Complex::new(0.0, 0.0); N]);

        // Collect iterator into existing buffer
        for (complex_in, complex_out) in spectrum_conjugate.zip(fixed_sized_spectrum.iter_mut()) {
            *complex_out = complex_in;
        }

        let samples = cfft_16384(&mut fixed_sized_spectrum);

        samples
            .iter()
            .map(|complex| complex.im / N as f32)
            .collect()
    };

    eframe::run_native(
        Box::new(Loid {
            audio_sink: Arc::new(sink),

            sample_rate,
            samples,
            reconstructed_samples,

            spectrum: half_spectrum,
        }),
        NativeOptions::default(),
    )
}

struct Loid {
    audio_sink: Arc<Sink>,

    sample_rate: u32,

    samples: Vec<f32>,
    reconstructed_samples: Vec<f32>,

    spectrum: Box<[Complex<f32>; N / 2]>,
}

impl Loid {
    fn play(&self, samples: &[f32], frame: Frame) {
        self.audio_sink
            .append(SamplesBuffer::new(1, self.sample_rate, samples));

        thread::spawn({
            let audio_sink = self.audio_sink.clone();
            move || {
                audio_sink.sleep_until_end();
                frame.request_repaint();
            }
        });
    }
}

impl App for Loid {
    fn update(&mut self, ctx: &CtxRef, frame: &Frame) {
        CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(self.audio_sink.empty(), Button::new("Play Original"))
                    .clicked()
                {
                    self.play(self.samples.as_ref(), frame.clone());
                }

                if ui
                    .add_enabled(self.audio_sink.empty(), Button::new("Play Reconstructed"))
                    .clicked()
                {
                    self.play(self.reconstructed_samples.as_ref(), frame.clone());
                }
            });

            // FIXME: heavy on the iterators I think
            Plot::new("samples")
                .height(ui.available_height() / 3.0)
                .center_y_axis(true)
                .show(ui, |ui| {
                    ui.points(
                        Points::new(Values::from_values_iter(
                            self.samples
                                .iter()
                                .copied()
                                .enumerate()
                                .map(|(n, x)| Value::new(n as f32 / self.sample_rate as f32, x)),
                        ))
                        .stems(0.0),
                    );

                    ui.vline(VLine::new(N as f32 / self.sample_rate as f32));
                    ui.vline(VLine::new(0.0 / self.sample_rate as f32));
                });

            Plot::new("frequencies")
                .height(ui.available_height() / 2.0)
                .legend(Legend::default())
                .show(ui, |ui| {
                    let amplitudes = self.spectrum.iter().map(|complex| complex.norm());

                    ui.bar_chart(
                        BarChart::new(
                            amplitudes
                                .enumerate()
                                .map(|(n, amp)| Bar::new(n as f64, amp as f64))
                                .collect(),
                        )
                        .name("Amplitude"),
                    );

                    let phases = self.spectrum.iter().map(|complex| complex.arg());

                    ui.bar_chart(
                        BarChart::new(
                            phases
                                .enumerate()
                                .map(|(n, amp)| Bar::new(n as f64, amp as f64))
                                .collect(),
                        )
                        .name("Phase"),
                    );
                });

            Plot::new("reconstructed_samples")
                .height(ui.available_height())
                .center_y_axis(true)
                .show(ui, |ui| {
                    ui.points(
                        Points::new(Values::from_values_iter(
                            self.reconstructed_samples
                                .iter()
                                .copied()
                                .enumerate()
                                .map(|(n, x)| Value::new(n as f32 / self.sample_rate as f32, x)),
                        ))
                        .stems(0.0),
                    );

                    ui.vline(VLine::new(N as f32 / self.sample_rate as f32));
                    ui.vline(VLine::new(0.0 / self.sample_rate as f32));
                });
        });
    }

    fn name(&self) -> &str {
        "Shitty Loid"
    }
}

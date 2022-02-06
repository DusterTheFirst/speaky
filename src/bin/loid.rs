use std::{
    iter,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use color_eyre::eyre::Context;
use eframe::{
    egui::{
        plot::{Bar, BarChart, Legend, Plot, Points, VLine, Value, Values},
        Button, CentralPanel, Color32, ComboBox, CtxRef, Slider,
    },
    epi::{App, Frame},
    NativeOptions,
};
use microfft::{complex::cfft_16384, real::rfft_16384};
use num_complex::Complex;
use rodio::{buffer::SamplesBuffer, OutputStream, Sink, Source};
use speaky::{
    install_tracing,
    tts::{load_language, setup_tts, synthesize},
};

const N: usize = 16384;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    install_tracing()?;

    let (_stream, stream_handle) = OutputStream::try_default().unwrap();

    let sink = Sink::try_new(&stream_handle).unwrap();

    let resources = load_language("en-US").unwrap();

    let mut engine = setup_tts(resources).wrap_err("unable to setup tts engine")?;

    let speech = synthesize(&mut engine, "Some Body Once").wrap_err("unable to synthesize text")?;

    let sample_rate = speech.sample_rate();
    let samples: Vec<f32> = speech.convert_samples().collect();

    let half_spectrum = {
        assert!(samples.len() >= N, "Too few samples");

        let mut fixed_sized_samples = Box::new([0.0; N]);
        fixed_sized_samples.copy_from_slice(&samples[..N]);

        let fixed_sized_samples = Box::leak(fixed_sized_samples);

        let spectrum = rfft_16384(fixed_sized_samples);

        // This saves a large copy
        unsafe { Box::from_raw(spectrum) }
    };

    let maximum = half_spectrum
        .iter()
        .enumerate()
        .map(|(freq, complex)| (freq, complex.norm_sqr()))
        .reduce(|(freq1, norm1), (freq2, norm2)| {
            if norm1 > norm2 {
                (freq1, norm1)
            } else {
                (freq2, norm2)
            }
        })
        .map(|(freq, _)| freq)
        .unwrap();

    dbg!(maximum);

    let scale = 2.0;

    let mut half_spectrum_rotate = Box::new([Complex::new(0.0, 0.0); N / 2]);
    half_spectrum_rotate[0] = half_spectrum[0];
    for (freq, component) in half_spectrum[1..].iter().copied().enumerate() {
        let new_freq = (freq as f32 * scale).round() as usize;

        if new_freq >= half_spectrum_rotate.len() {
            break;
        }

        half_spectrum_rotate[new_freq] = component;
    }

    let half_spectrum = half_spectrum_rotate;

    // let maximum = 440;

    // half_spectrum[1..].rotate_right(maximum);
    // half_spectrum[0].im = half_spectrum[1].re;
    // half_spectrum[1..maximum].fill(Complex::new(0.0, 0.0));

    let maximum = half_spectrum
        .iter()
        .enumerate()
        .map(|(freq, complex)| (freq, complex.norm_sqr()))
        .reduce(|(freq1, norm1), (freq2, norm2)| {
            if norm1 > norm2 {
                (freq1, norm1)
            } else {
                (freq2, norm2)
            }
        })
        .map(|(freq, _)| freq);

    dbg!(maximum);

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

            last_update: None,

            cursor: 0,
            width: N,
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

    last_update: Option<Duration>,

    cursor: usize,
    width: usize,
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
        let update_start = Instant::now();

        CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "Last frame: {:.4} ms",
                    frame.info().cpu_usage.unwrap_or(0.0) * 1000.0
                ));
                ui.label(format!(
                    "Last update: {:.4} ms",
                    self.last_update.unwrap_or_default().as_secs_f64() * 1000.0
                ));
            });

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

            ui.add(
                Slider::new(&mut self.cursor, 0..=(self.samples.len() - self.width - 1))
                    .prefix("sample ")
                    .text("FFT window start"),
            );
            ComboBox::from_label("FFT window width")
                .selected_text(format!("{} samples", self.width))
                .show_ui(ui, |ui| {
                    for width in 1..=14 {
                        let width = 1 << width;

                        ui.selectable_value(&mut self.width, width, format!("{width}"));
                    }
                });

            Plot::new("samples")
                .height(ui.available_height() / 2.0)
                .center_y_axis(true)
                .legend(Legend::default())
                .show(ui, |ui| {
                    ui.points(
                        Points::new(Values::from_values_iter(
                            self.samples
                                .iter()
                                .copied()
                                .enumerate()
                                .map(|(n, x)| Value::new(n as f32 / self.sample_rate as f32, x)),
                        ))
                        .name("Original Samples")
                        .stems(0.0),
                    );

                    // ui.points(
                    //     Points::new(Values::from_values_iter(
                    //         self.reconstructed_samples
                    //             .iter()
                    //             .copied()
                    //             .enumerate()
                    //             // .step_by(skip + 1)
                    //             .map(|(n, x)| Value::new(n as f32 / self.sample_rate as f32, x)),
                    //     ))
                    //     .name("Reconstructed Samples")
                    //     .stems(0.0),
                    // );

                    let starting_cursor = self.cursor as f32 / self.sample_rate as f32;
                    let ending_cursor = (self.cursor + self.width) as f32 / self.sample_rate as f32;

                    ui.vline(
                        VLine::new(starting_cursor)
                            .color(Color32::DARK_GREEN)
                            .width(2.5),
                    );
                    ui.vline(
                        VLine::new(ending_cursor)
                            .color(Color32::DARK_RED)
                            .width(1.5),
                    );
                });

            Plot::new("frequencies")
                .height(ui.available_height())
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
        });

        self.last_update.replace(update_start.elapsed());
    }

    fn name(&self) -> &str {
        "Shitty Loid"
    }
}

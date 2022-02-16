#![forbid(unsafe_code)]

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use cpal::SampleRate;
use eframe::{
    egui::{
        plot::{Bar, BarChart, Legend, Plot, PlotUi, Points, Text, VLine, Value, Values},
        Align2, Button, CentralPanel, Color32, ComboBox, CtxRef, InnerResponse, Slider, TextStyle,
    },
    epi::{App, Frame},
    NativeOptions,
};
use num_complex::Complex;
use rodio::{buffer::SamplesBuffer, source::SineWave, OutputStream, Sink, Source};
use speaky::{
    audio, install_tracing,
    spectrum::{reconstruct_samples, shift_spectrum, spectrum},
    tts::{load_language, setup_tts, synthesize},
};
use tracing::warn;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    install_tracing()?;

    let (_stream, stream_handle) = OutputStream::try_default().unwrap();

    let sink = Sink::try_new(&stream_handle).unwrap();

    // let resources = load_language("en-US").unwrap();

    // let mut engine = setup_tts(resources).wrap_err("unable to setup tts engine")?;

    // let speech = synthesize(&mut engine, "Some Body Once").wrap_err("unable to synthesize text")?;
    let speech = SineWave::new(440.0).take_duration(Duration::from_secs(2));

    let sample_rate = speech.sample_rate();
    let samples: Vec<f32> = speech.convert_samples().collect();

    // let (samples, SampleRate(sample_rate)) = audio::input::h()?;

    eframe::run_native(
        Box::new(Loid {
            audio_sink: Arc::new(sink),

            sample_rate,

            samples,
            reconstructed_samples: Vec::new(),
            reconstructed_window_samples: Vec::new(),
            reconstructed_work_buffer: Vec::new(),

            spectrum: Vec::new(),
            shifted_spectrum: Vec::new(),

            last_update: None,

            playback_head: Arc::new(AtomicUsize::new(0)),

            follow_playback: true,
            full_spectrum: true,
            phase: false,

            cursor: 0,
            width: 2048, // TODO: enum

            shift: 0.0,
            is_scale: false,
        }),
        NativeOptions::default(),
    )
}

struct Loid {
    audio_sink: Arc<Sink>,

    sample_rate: u32,

    samples: Vec<f32>,
    reconstructed_samples: Vec<f32>,
    reconstructed_window_samples: Vec<f32>,
    reconstructed_work_buffer: Vec<Complex<f32>>,

    spectrum: Vec<Complex<f32>>,
    shifted_spectrum: Vec<Complex<f32>>,

    last_update: Option<Duration>,

    playback_head: Arc<AtomicUsize>,

    follow_playback: bool,
    full_spectrum: bool,
    phase: bool,

    cursor: usize,
    width: usize,

    shift: f64,
    is_scale: bool,
}

impl Loid {
    fn reconstruct_samples(&mut self) {
        self.reconstructed_samples.clear();

        let mut window_samples = Vec::new();

        for window_start in (0..self.samples.len()).step_by(self.width) {
            if window_start + self.width >= self.samples.len() {
                let window = window_start..window_start + self.width;
                warn!(?window, "skipping window");

                break;
            }

            spectrum(window_start, self.width, &self.samples, &mut self.spectrum);
            if self.is_scale {
                todo!();
                // scale_spectrum(spectrum, &mut self.shifted_spectrum, self.shift);

                // self.shifted_spectrum[0] = Complex::new(0.0, 0.0);
            } else {
                shift_spectrum(
                    self.bucket_from_freq(self.shift),
                    &self.spectrum,
                    &mut self.shifted_spectrum,
                )
            }

            reconstruct_samples(
                &self.shifted_spectrum,
                &mut self.reconstructed_work_buffer,
                &mut window_samples,
                self.width,
            );

            self.reconstructed_samples.append(&mut window_samples);

            // self.shift += 500.0 * (self.width as f64 / self.samples.len() as f64) as f64;
        }
    }

    fn freq_from_bucket(&self, bucket: usize) -> f64 {
        bucket as f64 / self.width as f64 * self.sample_rate as f64
    }

    fn bucket_from_freq(&self, freq: f64) -> usize {
        ((freq * self.width as f64) / self.sample_rate as f64).round() as usize
    }

    fn play(&self, samples: &[f32], frame: Frame) {
        let duration = Duration::from_millis(10);

        let samples_per_duration =
            (self.sample_rate as f64 * duration.as_secs_f64()).round() as usize;

        self.audio_sink.append(
            SamplesBuffer::new(1, self.sample_rate, samples).periodic_access(duration, {
                let playback_head = self.playback_head.clone();
                let frame = frame.clone();

                playback_head.store(0, Ordering::SeqCst);

                move |_signal| {
                    playback_head.fetch_add(samples_per_duration, Ordering::SeqCst);
                    frame.request_repaint()
                }
            }),
        );

        thread::spawn({
            let audio_sink = self.audio_sink.clone();

            move || {
                audio_sink.sleep_until_end();
                frame.request_repaint();
            }
        });
    }

    fn display_spectrum(&self, ui: &mut PlotUi, spectrum: &[Complex<f32>], title: &str) {
        let spectrum = if self.full_spectrum {
            spectrum
        } else {
            &spectrum[..self.width / 2]
        };

        let map_arg: &dyn Fn(&Complex<f32>) -> f32 = &|complex| complex.arg();
        let map_norm: &dyn Fn(&Complex<f32>) -> f32 = &|complex| complex.norm() / self.width as f32;

        let amplitudes = spectrum
            .iter()
            .map(if self.phase { map_arg } else { map_norm })
            .enumerate();

        ui.bar_chart(
            BarChart::new(
                amplitudes
                    .clone()
                    .map(|(n, amp)| Bar::new(self.freq_from_bucket(n), amp as f64))
                    .collect(),
            )
            .width(self.freq_from_bucket(1))
            .name(&title),
        );

        if !self.phase {
            if let Some((bucket, max)) =
                amplitudes
                    .take(self.width / 2)
                    .reduce(|one, two| if one.1 > two.1 { one } else { two })
            {
                ui.text(
                    Text::new(
                        Value::new(self.freq_from_bucket(bucket), max),
                        format!("{:.2}Hz", self.freq_from_bucket(bucket)),
                    )
                    .style(TextStyle::Monospace)
                    .anchor(Align2::CENTER_BOTTOM),
                )
            }
        }
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
                ui.label(format!(
                    "Max refresh: {:.1} fps",
                    1.0 / frame.info().cpu_usage.unwrap_or(0.0)
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
                    .add_enabled(
                        self.audio_sink.empty() && !self.reconstructed_samples.is_empty(),
                        Button::new("Play Reconstructed"),
                    )
                    .clicked()
                {
                    self.play(self.reconstructed_samples.as_ref(), frame.clone());
                }

                ui.checkbox(&mut self.follow_playback, "FFT follows playback");
            });

            let InnerResponse { inner: cursor, .. } = ui.horizontal_wrapped(|ui| {
                ComboBox::from_label("FFT window width")
                    .selected_text(format!("{} samples", self.width))
                    .show_ui(ui, |ui| {
                        for width in 1..=14 {
                            let width = 1 << width;

                            ui.selectable_value(&mut self.width, width, format!("{width}"));
                        }
                    });

                let max_cursor = self.samples.len() - self.width - 1;
                self.cursor = self.cursor.min(max_cursor);

                ui.add_enabled_ui(!self.follow_playback || self.audio_sink.empty(), |ui| {
                    ui.add(
                        Slider::new(&mut self.cursor, 0..=max_cursor)
                            .integer()
                            .prefix("sample ")
                            .text("FFT window start"),
                    );

                    if ui.button("Previous").clicked() {
                        self.cursor -= self.width;
                    }

                    if ui.button("Next").clicked() {
                        self.cursor += self.width;
                    }
                });

                if self.follow_playback && !self.audio_sink.empty() {
                    self.playback_head.load(Ordering::SeqCst).min(max_cursor)
                } else {
                    self.cursor
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.is_scale, "Use scaling");

                ui.add(
                    Slider::new(&mut self.shift, 0.0..=1000.0)
                        .suffix(" Hz")
                        .text("Frequency shift"),
                );

                if ui.button("Reconstruct Samples").clicked() {
                    self.reconstruct_samples();
                }
            });

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.full_spectrum, "Show full spectrum");
                ui.checkbox(&mut self.phase, "Show phase");
            });

            Plot::new("samples")
                .height(ui.available_height() / 3.0)
                .center_y_axis(true)
                .legend(Legend::default())
                .include_y(1.0)
                .include_y(-1.0)
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

                    ui.points(
                        Points::new(Values::from_values_iter(
                            self.reconstructed_samples
                                .iter()
                                .copied()
                                .enumerate()
                                .map(|(n, x)| Value::new(n as f32 / self.sample_rate as f32, x)),
                        ))
                        .name("Reconstructed Samples")
                        .stems(0.0),
                    );

                    ui.vline(
                        VLine::new(cursor as f32 / self.sample_rate as f32)
                            .color(Color32::DARK_GREEN)
                            .width(2.5),
                    );
                    ui.vline(
                        VLine::new((cursor + self.width) as f32 / self.sample_rate as f32)
                            .color(Color32::DARK_RED)
                            .width(1.5),
                    );
                    ui.vline(VLine::new(
                        self.playback_head
                            .load(Ordering::SeqCst)
                            .min(self.samples.len()) as f32
                            / self.sample_rate as f32,
                    ));
                });

            spectrum(
                cursor,
                self.width,
                self.samples.as_ref(),
                &mut self.spectrum,
            );
            // TODO: solve problem where you can use the shifted spectrum before calculation
            if self.is_scale {
                todo!();
                // scale_spectrum(&self.spectrum, &mut self.shifted_spectrum, self.shift);
            } else {
                shift_spectrum(
                    self.bucket_from_freq(self.shift),
                    &self.spectrum,
                    &mut self.shifted_spectrum,
                )
            }

            Plot::new("window_samples")
                .height(ui.available_height() / 2.0)
                .center_y_axis(true)
                .legend(Legend::default())
                .include_y(1.0)
                .include_y(-1.0)
                .show(ui, |ui| {
                    ui.points(
                        Points::new(Values::from_ys_f32(
                            &self.samples[cursor..(cursor + self.width)],
                        ))
                        .name("Samples")
                        .stems(0.0),
                    );

                    reconstruct_samples(
                        &self.shifted_spectrum,
                        &mut self.reconstructed_work_buffer,
                        &mut self.reconstructed_window_samples,
                        self.width,
                    );

                    ui.points(
                        Points::new(Values::from_ys_f32(&self.reconstructed_window_samples))
                            .name("Shifted Sample")
                            .stems(0.0),
                    );
                });

            Plot::new("frequencies")
                .height(ui.available_height())
                .legend(Legend::default())
                .include_y(0.2)
                .show(ui, |ui| {
                    self.display_spectrum(ui, &self.spectrum, "Original");

                    self.display_spectrum(ui, &self.shifted_spectrum, "Shifted");

                    if self.full_spectrum {
                        ui.vline(VLine::new(self.freq_from_bucket(self.width / 2)))
                    }
                });
        });

        self.last_update.replace(update_start.elapsed());
    }

    fn name(&self) -> &str {
        "Shitty Loid"
    }
}

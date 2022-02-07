#![forbid(unsafe_code)]

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use color_eyre::eyre::Context;
use eframe::{
    egui::{
        plot::{Bar, BarChart, Legend, Plot, Points, Text, VLine, Value, Values},
        Align2, Button, CentralPanel, Color32, ComboBox, CtxRef, InnerResponse, Slider, TextStyle,
    },
    epi::{App, Frame},
    NativeOptions,
};
use rodio::{buffer::SamplesBuffer, OutputStream, Sink, Source};
use speaky::{
    install_tracing,
    spectrum::{full_spectrum, reconstruct_samples, shift_spectrum, spectrum},
    tts::{load_language, setup_tts, synthesize},
};

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

    eframe::run_native(
        Box::new(Loid {
            audio_sink: Arc::new(sink),

            sample_rate,
            samples,

            last_update: None,

            playback_head: Arc::new(AtomicUsize::new(0)),

            follow_playback: true,

            cursor: 0,
            width: 2048, // TODO: enum

            shift: 1.0,
        }),
        NativeOptions::default(),
    )
}

struct Loid {
    audio_sink: Arc<Sink>,

    sample_rate: u32,

    samples: Vec<f32>,

    last_update: Option<Duration>,

    playback_head: Arc<AtomicUsize>,

    follow_playback: bool,

    cursor: usize,
    width: usize,

    shift: f32,
}

impl Loid {
    fn freq_from_bucket(&self, bucket: usize) -> f64 {
        bucket as f64 / self.width as f64 * self.sample_rate as f64
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

                ui.add_enabled(
                    !self.follow_playback || self.audio_sink.empty(),
                    Slider::new(&mut self.cursor, 0..=max_cursor)
                        .integer()
                        .prefix("sample ")
                        .text("FFT window start"),
                );

                if self.follow_playback && !self.audio_sink.empty() {
                    self.playback_head.load(Ordering::SeqCst).min(max_cursor)
                } else {
                    self.cursor
                }
            });

            ui.add(Slider::new(&mut self.shift, 0.0..=5.0).text("Frequency shift"));

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

            let spectrum = spectrum(self.samples.as_ref(), cursor, self.width);
            let shifted_spectrum = shift_spectrum(&spectrum, self.shift);

            Plot::new("window_samples")
                .height(ui.available_height() / 2.0)
                .center_y_axis(true)
                .legend(Legend::default())
                .show(ui, |ui| {
                    ui.points(
                        Points::new(Values::from_ys_f32(
                            &self.samples[cursor..(cursor + self.width)],
                        ))
                        .name("Samples")
                        .stems(0.0),
                    );

                    let full_spectrum = full_spectrum(&shifted_spectrum);
                    let samples = reconstruct_samples(&full_spectrum, self.width);

                    ui.points(
                        Points::new(Values::from_ys_f32(&samples))
                            .name("Shifted Sample")
                            .stems(0.0),
                    );
                });

            Plot::new("frequencies")
                .height(ui.available_height())
                .legend(Legend::default())
                .include_y(0.2)
                .show(ui, |ui| {
                    {
                        let amplitudes = spectrum
                            .iter()
                            .map(|complex| complex.norm() / self.width as f32);

                        ui.bar_chart(
                            BarChart::new(
                                amplitudes
                                    .clone()
                                    .enumerate()
                                    .map(|(n, amp)| Bar::new(self.freq_from_bucket(n), amp as f64))
                                    .collect(),
                            )
                            .width(self.freq_from_bucket(1))
                            .name("Amplitude"),
                        );

                        if let Some((bucket, max)) =
                            amplitudes.enumerate().reduce(
                                |one, two| {
                                    if one.1 > two.1 {
                                        one
                                    } else {
                                        two
                                    }
                                },
                            )
                        {
                            ui.text(
                                Text::new(
                                    Value::new(self.freq_from_bucket(bucket), max),
                                    format!("{:.2}Hz", self.freq_from_bucket(bucket)),
                                )
                                .style(TextStyle::Monospace)
                                .anchor(Align2::CENTER_BOTTOM)
                                .name("Maximum"),
                            )
                        }
                    }
                    {
                        let amplitudes = shifted_spectrum
                            .iter()
                            .map(|complex| complex.norm() / self.width as f32);

                        ui.bar_chart(
                            BarChart::new(
                                amplitudes
                                    .clone()
                                    .enumerate()
                                    .map(|(n, amp)| Bar::new(self.freq_from_bucket(n), amp as f64))
                                    .collect(),
                            )
                            .width(self.freq_from_bucket(1))
                            .name("Shifted Amplitude"),
                        );

                        if let Some((bucket, max)) =
                            amplitudes.enumerate().reduce(
                                |one, two| {
                                    if one.1 > two.1 {
                                        one
                                    } else {
                                        two
                                    }
                                },
                            )
                        {
                            ui.text(
                                Text::new(
                                    Value::new(self.freq_from_bucket(bucket), max),
                                    format!("{:.2}Hz", self.freq_from_bucket(bucket)),
                                )
                                .style(TextStyle::Monospace)
                                .anchor(Align2::CENTER_BOTTOM)
                                .name("Shifted Maximum"),
                            )
                        }
                    }
                });
        });

        self.last_update.replace(update_start.elapsed());
    }

    fn name(&self) -> &str {
        "Shitty Loid"
    }
}

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
        plot::{Bar, BarChart, Legend, Plot, PlotUi, Points, Text, VLine, Value, Values},
        Align2, Button, CentralPanel, Color32, CtxRef, Label, SidePanel, Slider, TextStyle,
        TopBottomPanel,
    },
    epi::{App, Frame},
    NativeOptions,
};
use rodio::{buffer::SamplesBuffer, OutputStream, Sink, Source};
use speaky::{
    install_tracing,
    spectrum::{Spectrum, Waveform, WaveformAnalyzer},
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
    // let speech = SineWave::new(440.0).take_duration(Duration::from_secs(2));

    let sample_rate = speech.sample_rate();
    let samples: Vec<f32> = speech.convert_samples().collect();

    // let (samples, SampleRate(sample_rate)) = audio::input::h()?;

    eframe::run_native(
        Box::new(Loid {
            audio_sink: Arc::new(sink),

            waveform: Waveform::new(samples, sample_rate),
            analyzer: WaveformAnalyzer::default(),

            last_update: None,

            playback_head: Arc::new(AtomicUsize::new(0)),

            follow_playback: true,
            full_spectrum: false,
            phase: false,

            cursor: 0,
            width: 2048,

            shift: 0.0,
        }),
        NativeOptions::default(),
    )
}

struct Loid {
    audio_sink: Arc<Sink>,

    waveform: Waveform,
    analyzer: WaveformAnalyzer,

    last_update: Option<Duration>,

    playback_head: Arc<AtomicUsize>,

    follow_playback: bool,
    full_spectrum: bool,
    phase: bool,

    cursor: usize,
    width: usize,

    shift: f64,
}

impl Loid {
    // fn reconstruct_samples(&mut self) {
    //     self.reconstructed_samples.clear();

    //     let mut window_samples = Vec::new();

    //     for window_start in (0..self.samples.len()).step_by(self.width) {
    //         if window_start + self.width >= self.samples.len() {
    //             let window = window_start..window_start + self.width;
    //             warn!(?window, "skipping window");

    //             break;
    //         }

    //         spectrum(window_start, self.width, &self.samples, &mut self.spectrum);
    //         if self.is_scale {
    //             todo!();
    //             // scale_spectrum(spectrum, &mut self.shifted_spectrum, self.shift);

    //             // self.shifted_spectrum[0] = Complex::new(0.0, 0.0);
    //         } else {
    //             shift_spectrum(
    //                 self.bucket_from_freq(self.shift),
    //                 &self.spectrum,
    //                 &mut self.shifted_spectrum,
    //             )
    //         }

    //         reconstruct_samples(
    //             &self.shifted_spectrum,
    //             &mut self.reconstructed_work_buffer,
    //             &mut window_samples,
    //             self.width,
    //         );

    //         self.reconstructed_samples.append(&mut window_samples);

    //         // self.shift += 500.0 * (self.width as f64 / self.samples.len() as f64) as f64;
    //     }
    // }

    fn play(&self, samples: &[f32], frame: Frame) {
        let duration = Duration::from_millis(10);

        let samples_per_duration =
            (self.waveform.sample_rate() as f64 * duration.as_secs_f64()).round() as usize;

        self.audio_sink.append(
            SamplesBuffer::new(1, self.waveform.sample_rate(), samples).periodic_access(
                duration,
                {
                    let playback_head = self.playback_head.clone();
                    let frame = frame.clone();

                    playback_head.store(0, Ordering::SeqCst);

                    move |_signal| {
                        playback_head.fetch_add(samples_per_duration, Ordering::SeqCst);
                        frame.request_repaint()
                    }
                },
            ),
        );

        thread::spawn({
            let audio_sink = self.audio_sink.clone();

            move || {
                audio_sink.sleep_until_end();
                frame.request_repaint();
            }
        });
    }

    fn display_spectrum(
        ui: &mut PlotUi,
        spectrum: &Spectrum,
        title: &str,
        full_spectrum: bool,
        phase: bool,
    ) {
        let width = if full_spectrum {
            spectrum.width()
        } else {
            spectrum.width() / 2
        };

        let mut max = None;

        let buckets = if phase {
            spectrum
                .phases()
                .enumerate()
                .map(|(bucket, freq)| Bar::new(spectrum.freq_from_bucket(bucket), freq as f64))
                .take(width)
                .collect()
        } else {
            spectrum
                .amplitudes()
                .enumerate()
                .inspect(|new| {
                    if new.1 > max.get_or_insert(*new).1 {
                        max = Some(*new);
                    }
                })
                .map(|(bucket, amp)| Bar::new(spectrum.freq_from_bucket(bucket), amp as f64))
                .take(width)
                .collect()
        };

        ui.bar_chart(
            BarChart::new(buckets)
                .width(spectrum.freq_from_bucket(1))
                .name(&title),
        );

        if !phase {
            if let Some((bucket, max)) = max {
                let freq = spectrum.freq_from_bucket(bucket);

                ui.text(
                    Text::new(Value::new(freq, max), format!("{:.2}Hz", freq))
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

        SidePanel::left("left_panel").show(ctx, |ui| {
            ui.heading("Rendering Statistics");
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    Label::new(format!(
                        "Last frame: {:.4} ms",
                        frame.info().cpu_usage.unwrap_or(0.0) * 1000.0
                    ))
                    .wrap(false),
                );
                ui.add(
                    Label::new(format!(
                        "Last update: {:.4} ms",
                        self.last_update.unwrap_or_default().as_secs_f64() * 1000.0
                    ))
                    .wrap(false),
                );
                ui.add(
                    Label::new(format!(
                        "Max refresh: {:.1} fps",
                        1.0 / frame.info().cpu_usage.unwrap_or(0.0)
                    ))
                    .wrap(false),
                );
            });

            ui.separator();
            ui.heading("Playback");
            if ui
                .add_enabled(self.audio_sink.empty(), Button::new("Play Original"))
                .clicked()
            {
                self.play(self.waveform.samples(), frame.clone());
            }

            if ui
                .add_enabled(
                    false,
                    // self.audio_sink.empty() && !self.reconstructed_samples.is_empty(),
                    Button::new("Play Reconstructed"),
                )
                .clicked()
            {
                // self.play(self.reconstructed_samples.as_ref(), frame.clone());
            }

            if ui
                .add_enabled(false, Button::new("Reconstruct Samples"))
                .clicked()
            {
                // self.reconstruct_samples();
            }

            ui.checkbox(&mut self.follow_playback, "FFT follows playback");

            ui.separator();
            ui.add_enabled_ui(!self.follow_playback || self.audio_sink.empty(), |ui| {
                ui.heading("FFT");
                ui.label("Window Width");
                ui.add(
                    Slider::new(&mut self.width, 2..=1 << 14)
                        .logarithmic(true)
                        .suffix(" samples"),
                );

                let max_cursor = self.waveform.len() - self.width - 1;
                self.cursor = self.cursor.min(max_cursor);

                ui.label("Window Start");
                ui.add(Slider::new(&mut self.cursor, 0..=max_cursor).prefix("sample "));

                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(self.cursor >= self.width, Button::new("Previous"))
                        .clicked()
                    {
                        self.cursor -= self.width;
                    }

                    if ui
                        .add_enabled(
                            self.cursor + self.width * 2 <= self.waveform.len(),
                            Button::new("Next"),
                        )
                        .clicked()
                    {
                        self.cursor += self.width;
                    }
                });

                ui.separator();
                ui.heading("DSP");
                ui.label("Frequency shift");
                ui.add(Slider::new(&mut self.shift, 0.0..=1000.0).suffix(" Hz"));
            });

            ui.separator();
            ui.heading("Visualization");
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.full_spectrum, "Show full spectrum");
                ui.checkbox(&mut self.phase, "Show phase");
            });
        });

        let cursor = if self.follow_playback && !self.audio_sink.empty() {
            self.playback_head
                .load(Ordering::SeqCst)
                .min(self.waveform.len() - self.width - 1)
        } else {
            self.cursor
        };

        let range = cursor..(cursor + self.width);
        let mut spectrum = self.analyzer.spectrum(&self.waveform, range.clone());

        TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.label(format!(
                "Frequency Resolution: {} Hz",
                spectrum.freq_from_bucket(1)
            ));

            ui.label(format!(
                "FFT algorithm: cfft_{}",
                self.width.next_power_of_two()
            ));
        });

        CentralPanel::default().show(ctx, |ui| {
            Plot::new("samples")
                .height(ui.available_height() / 3.0)
                .center_y_axis(true)
                .legend(Legend::default())
                .include_y(1.0)
                .include_y(-1.0)
                .show(ui, |ui| {
                    ui.points(
                        Points::new(Values::from_values_iter(
                            self.waveform.time_domain().map(|(x, y)| Value::new(x, y)),
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
                    //             .map(|(n, x)| Value::new(n as f32 / self.sample_rate as f32, x)),
                    //     ))
                    //     .name("Reconstructed Samples")
                    //     .stems(0.0),
                    // );

                    ui.vline(
                        VLine::new(self.waveform.time_from_sample(cursor))
                            .color(Color32::DARK_GREEN)
                            .width(2.5),
                    );
                    ui.vline(
                        VLine::new(self.waveform.time_from_sample(cursor + self.width))
                            .color(Color32::DARK_RED)
                            .width(1.5),
                    );
                    ui.vline(VLine::new(
                        self.waveform
                            .time_from_sample(self.playback_head.load(Ordering::SeqCst)),
                    ));
                });

            // spectrum(
            //     cursor,
            //     self.width,
            //     self.samples.as_ref(),
            //     &mut self.spectrum,
            // );
            // TODO: solve problem where you can use the shifted spectrum before calculation
            // shift_spectrum(
            //     self.bucket_from_freq(self.shift),
            //     &self.spectrum,
            //     &mut self.shifted_spectrum,
            // )

            Plot::new("window_samples")
                .height(ui.available_height() / 2.0)
                .center_y_axis(true)
                .legend(Legend::default())
                .include_y(1.0)
                .include_y(-1.0)
                .show(ui, |ui| {
                    ui.points(
                        Points::new(Values::from_ys_f32(&self.waveform.samples()[range.clone()]))
                            .name("Samples")
                            .stems(0.0),
                    );

                    // reconstruct_samples(
                    //     &self.shifted_spectrum,
                    //     &mut self.reconstructed_work_buffer,
                    //     &mut self.reconstructed_window_samples,
                    //     self.width,
                    // );

                    // ui.points(
                    //     Points::new(Values::from_ys_f32(&self.reconstructed_window_samples))
                    //         .name("Shifted Sample")
                    //         .stems(0.0),
                    // );
                });

            Plot::new("frequencies")
                .height(ui.available_height())
                .legend(Legend::default())
                .include_y(0.2)
                .show(ui, |ui| {
                    Self::display_spectrum(
                        ui,
                        &spectrum,
                        "Original",
                        self.full_spectrum,
                        self.phase,
                    );

                    spectrum.shift(spectrum.bucket_from_freq(self.shift));

                    Self::display_spectrum(
                        ui,
                        &spectrum,
                        "Shifted",
                        self.full_spectrum,
                        self.phase,
                    );

                    if self.full_spectrum {
                        ui.vline(VLine::new(spectrum.freq_from_bucket(spectrum.width() / 2)))
                    }
                });
        });

        self.last_update.replace(update_start.elapsed());
    }

    fn name(&self) -> &str {
        "Shitty Loid"
    }
}

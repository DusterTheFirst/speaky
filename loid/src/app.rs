#![forbid(unsafe_code)]

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use common::{
    color_eyre,
    rodio::{buffer::SamplesBuffer, source::SineWave, OutputStream, Sink, Source},
    spectrum::{Spectrum, Waveform, Window},
};
use eframe::{
    egui::{
        plot::{Bar, BarChart, Legend, Line, Plot, PlotUi, Points, Text, VLine, Value, Values},
        Align2, Button, CentralPanel, Color32, CtxRef, Label, ScrollArea, SidePanel, Slider,
        TextStyle, TopBottomPanel,
    },
    epi::{App, Frame},
};
use instant::Instant;

pub struct Application {
    math_elapsed: Duration,

    audio_sink: Arc<Sink>,

    waveform: Waveform<'static>,

    window: Window,

    playback_head: Arc<AtomicUsize>,

    follow_playback: bool,
    full_spectrum: bool,
    phase: bool,
    decibels: bool,
    line: bool,
    stems: bool,

    cursor: usize,
    fft_width: u8,
    window_width: usize,
    hop_frac: usize,

    shift: f64,
}

impl Application {
    pub fn initialize() -> color_eyre::Result<Self> {
        let (_stream, stream_handle) = OutputStream::try_default().unwrap();

        let sink = Sink::try_new(&stream_handle).unwrap();

        // let resources = load_language("en-US").unwrap();

        // let mut engine = setup_tts(resources).wrap_err("unable to setup tts engine")?;

        // let speech = synthesize(&mut engine, "Some Body Once").wrap_err("unable to synthesize text")?;
        let speech = SineWave::new(120.0).take_duration(Duration::from_millis(300));

        let sample_rate = speech.sample_rate();
        let samples: Vec<f32> = speech.convert_samples().collect();

        // let (samples, SampleRate(sample_rate)) = audio::input::h()?;

        Ok(Application {
            math_elapsed: Duration::ZERO,

            audio_sink: Arc::new(sink),

            waveform: Waveform::new(samples, sample_rate),

            window: Window::Hann,

            playback_head: Arc::new(AtomicUsize::new(0)),

            follow_playback: true,
            full_spectrum: false,
            phase: false,
            decibels: false,
            line: false,
            stems: true,

            cursor: 0,
            fft_width: 11,
            window_width: 2048,
            hop_frac: 4,

            shift: 0.0,
        })
    }

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
        decibels: bool,
    ) {
        // TODO: DECIBELS

        #[inline(always)]
        fn map(
            iterator: impl Iterator<Item = f32>,
            freq: impl Fn(usize) -> f64,
            db: impl Fn(f32) -> f32,
        ) -> Vec<Bar> {
            iterator
                .enumerate()
                .map(|(bucket, mag)| Bar::new(freq(bucket), db(mag) as f64))
                .collect()
        }

        let db = |mag: f32| -> f32 {
            if decibels {
                20.0 * if mag == 0.0 { 0.0 } else { mag.log10() }
            } else {
                mag
            }
        };

        let freq = |b| spectrum.freq_from_bucket(b);

        let buckets = match (phase, full_spectrum) {
            (true, true) => map(&mut spectrum.phases(), freq, db),
            (true, false) => map(spectrum.phases_real(), freq, db),
            (false, true) => map(spectrum.amplitudes(), freq, db),
            (false, false) => map(spectrum.amplitudes_real(), freq, db),
        };

        ui.bar_chart(
            BarChart::new(buckets)
                .width(spectrum.freq_resolution())
                .name(&title),
        );

        if !phase {
            if let Some((bucket, max)) = spectrum.main_frequency() {
                let freq = spectrum.freq_from_bucket(bucket);

                ui.text(
                    Text::new(Value::new(freq, db(max)), format!("{:.2}Hz", freq))
                        .style(TextStyle::Monospace)
                        .anchor(Align2::CENTER_BOTTOM),
                )
            }
        }
    }
}

impl App for Application {
    fn update(&mut self, ctx: &CtxRef, frame: &Frame) {
        SidePanel::left("left_panel").show(ctx, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Rendering Statistics");
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        Label::new(format!(
                            "Last math: {:.4} ms",
                            self.math_elapsed.as_millis()
                        ))
                        .wrap(false),
                    );
                    ui.add(
                        Label::new(format!(
                            "Last frame: {:.4} ms",
                            frame.info().cpu_usage.unwrap_or(0.0) * 1000.0
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
                    ui.label("FFT Width");
                    ui.add(
                        Slider::new(
                            &mut self.fft_width,
                            1..=(self.waveform.len().next_power_of_two().trailing_zeros() as u8
                                - 1),
                        )
                        .prefix("2^")
                        .suffix(" samples"),
                    );

                    // Ensure the window width is always <= fft_width
                    self.window_width = self.window_width.min(1 << self.fft_width);

                    ui.label("Window Width");
                    ui.add(
                        Slider::new(&mut self.window_width, 2..=(1 << self.fft_width))
                            .suffix(" samples"),
                    );

                    ui.label("Window Function");
                    ui.horizontal_wrapped(|ui| {
                        for window in Window::ALL {
                            ui.selectable_value(&mut self.window, window, window.to_string());
                        }
                    });

                    ui.label("Hop Fraction");
                    ui.add(
                        Slider::new(&mut self.hop_frac, 1..=16)
                            .prefix("1/")
                            .logarithmic(true),
                    );

                    let max_cursor = self.waveform.len() - (1 << self.fft_width) - 1;
                    self.cursor = self.cursor.min(max_cursor);

                    ui.label("Window Start");
                    ui.add(Slider::new(&mut self.cursor, 0..=max_cursor).prefix("sample "));

                    ui.horizontal_wrapped(|ui| {
                        let step = self.window_width / self.hop_frac;

                        if ui
                            .add_enabled(self.cursor >= step, Button::new("Previous"))
                            .clicked()
                        {
                            self.cursor -= step;
                        }

                        if ui
                            .add_enabled(
                                self.cursor + self.window_width + step <= self.waveform.len(),
                                Button::new("Next"),
                            )
                            .clicked()
                        {
                            self.cursor += step;
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
                    ui.checkbox(&mut self.decibels, "Decibels");
                    ui.checkbox(&mut self.line, "Line Plot");
                    ui.checkbox(&mut self.stems, "Stems");
                });

                ui.separator();
                ui.heading("Debug");
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Panic").clicked() {
                        panic!("User initiated panic");
                    }
                });
            });
        });

        let cursor = if self.follow_playback && !self.audio_sink.empty() {
            self.playback_head
                .load(Ordering::SeqCst)
                .min(self.waveform.len() - self.window_width - 1)
        } else {
            self.cursor
        };

        // Calculate FFT width in bytes
        let fft_width = 1 << self.fft_width;

        let math_start = Instant::now();

        // Get the slice of the waveform to work on
        let waveform = self.waveform.slice(cursor..(cursor + self.window_width));

        // Get the frequency spectrum of the waveform
        let spectrum = waveform.spectrum(self.window, fft_width);

        // Shift the spectrum
        let shifted_spectrum = spectrum.shift(spectrum.bucket_from_freq(self.shift));

        let reconstructed = shifted_spectrum.waveform();
        let reconstructed = reconstructed.slice(..self.window_width);

        self.math_elapsed = math_start.elapsed();

        TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.label(format!(
                "Frequency Resolution: {} Hz",
                spectrum.freq_resolution()
            ));

            ui.label(format!("FFT algorithm: cfft_{}", fft_width));
        });

        CentralPanel::default().show(ctx, |ui| {
            let point_line = |ui: &mut PlotUi, name: &str, series: Values| {
                if self.line {
                    let line = Line::new(series).name(name);

                    ui.line(if self.stems { line.fill(0.0) } else { line });
                } else {
                    let points = Points::new(series).name(name);

                    ui.points(if self.stems {
                        points.stems(0.0)
                    } else {
                        points
                    });
                }
            };

            Plot::new("samples")
                .height(ui.available_height() / 3.0)
                .center_y_axis(true)
                .legend(Legend::default())
                .include_y(1.0)
                .include_y(-1.0)
                .show(ui, |ui| {
                    point_line(
                        ui,
                        "Original waveform",
                        Values::from_values_iter(
                            self.waveform.time_domain().map(|(x, y)| Value::new(x, y)),
                        ),
                    );

                    // TODO:
                    // ui.points(
                    //     Points::new(Values::from_values_iter(
                    //         reconstructed.time_domain().map(|(x, y)| Value::new(x, y)),
                    //     ))
                    //     .name("Reconstructed Samples")
                    //     .stems(0.0),
                    // );

                    ui.vline(
                        VLine::new(self.waveform.time_from_sample(cursor))
                            .color(Color32::DARK_GREEN)
                            .width(2.5)
                            .name("Start of window"),
                    );
                    ui.vline(
                        VLine::new(self.waveform.time_from_sample(cursor + self.window_width))
                            .color(Color32::DARK_RED)
                            .width(1.5)
                            .name("End of window"),
                    );
                    ui.vline(
                        VLine::new(
                            self.waveform
                                .time_from_sample(cursor + self.window_width / self.hop_frac),
                        )
                        .color(Color32::GOLD)
                        .name("Start of next window"),
                    );
                    ui.vline(
                        VLine::new(
                            self.waveform
                                .time_from_sample(self.playback_head.load(Ordering::SeqCst)),
                        )
                        .color(Color32::LIGHT_BLUE)
                        .name("Playback head"),
                    );
                });

            Plot::new("window_samples")
                .height(ui.available_height() / 2.0)
                .center_y_axis(true)
                .legend(Legend::default())
                .include_y(1.0)
                .include_y(-1.0)
                .show(ui, |ui| {
                    point_line(
                        ui,
                        "Original samples",
                        Values::from_ys_f32(waveform.samples()),
                    );

                    ui.line(
                        Line::new(Values::from_values_iter(
                            self.window
                                .into_iter(self.window_width)
                                .enumerate()
                                .map(|(i, w)| Value::new(i as f32, w)),
                        ))
                        .name("Window"),
                    );

                    point_line(
                        ui,
                        "Windowed samples",
                        Values::from_values_iter(
                            waveform
                                .samples()
                                .iter()
                                .zip(self.window.into_iter(self.window_width))
                                .enumerate()
                                .map(|(i, (sample, w))| Value::new(i as f32, w * sample)),
                        ),
                    );

                    point_line(
                        ui,
                        "Shifted samples",
                        Values::from_ys_f32(reconstructed.samples()),
                    );

                    ui.vline(
                        VLine::new((self.window_width / self.hop_frac) as f32)
                            .name("Start of next window"),
                    );
                });

            Plot::new("frequencies")
                .height(ui.available_height())
                .legend(Legend::default())
                .center_y_axis(true)
                .include_x(fft_width as f64)
                .show(ui, |ui| {
                    Self::display_spectrum(
                        ui,
                        &spectrum,
                        "Frequency spectrum",
                        self.full_spectrum,
                        self.phase,
                        self.decibels,
                    );

                    Self::display_spectrum(
                        ui,
                        &shifted_spectrum,
                        "Shifted frequency spectrum",
                        self.full_spectrum,
                        self.phase,
                        self.decibels,
                    );
                });
        });
    }

    fn name(&self) -> &str {
        "Fun with FFT"
    }
}

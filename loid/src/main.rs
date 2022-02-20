#![forbid(unsafe_code)]

use std::{
    iter,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use common::{
    color_eyre, install_tracing,
    rodio::{buffer::SamplesBuffer, source::SineWave, OutputStream, Sink, Source},
    spectrum::{Spectrum, Waveform, WaveformAnalyzer, Window},
};
use eframe::{
    egui::{
        plot::{Bar, BarChart, Legend, Line, Plot, PlotUi, Points, Text, VLine, Value, Values},
        Align2, Button, CentralPanel, Color32, CtxRef, Label, ScrollArea, SidePanel, Slider,
        TextStyle, TopBottomPanel,
    },
    epi::{App, Frame},
    NativeOptions,
};

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    install_tracing()?;

    let (_stream, stream_handle) = OutputStream::try_default().unwrap();

    let sink = Sink::try_new(&stream_handle).unwrap();

    // let resources = load_language("en-US").unwrap();

    // let mut engine = setup_tts(resources).wrap_err("unable to setup tts engine")?;

    // let speech = synthesize(&mut engine, "Some Body Once").wrap_err("unable to synthesize text")?;
    let speech = SineWave::new(120.0).take_duration(Duration::from_secs(1));

    let sample_rate = speech.sample_rate();
    let samples: Vec<f32> = speech.convert_samples().collect();

    // let (samples, SampleRate(sample_rate)) = audio::input::h()?;

    eframe::run_native(
        Box::new(Loid {
            audio_sink: Arc::new(sink),

            waveform: Waveform::new(samples, sample_rate),
            analyzer: WaveformAnalyzer::default(),

            window: Window::Hann,

            playback_head: Arc::new(AtomicUsize::new(0)),

            follow_playback: true,
            full_spectrum: false,
            phase: false,

            cursor: 0,
            fft_width: 11,
            window_width: 2048,
            hop_frac: 4,

            shift: 0.0,
        }),
        NativeOptions::default(),
    )
}

struct Loid {
    audio_sink: Arc<Sink>,

    waveform: Waveform<'static>,
    analyzer: WaveformAnalyzer,

    window: Window,

    playback_head: Arc<AtomicUsize>,

    follow_playback: bool,
    full_spectrum: bool,
    phase: bool,

    cursor: usize,
    fft_width: u8,
    window_width: usize,
    hop_frac: usize,

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
        // TODO: DECIBELS

        #[inline(always)]
        fn map(
            iterator: impl Iterator<Item = f32>,
            freq_from_bucket: impl Fn(usize) -> f64,
        ) -> Vec<Bar> {
            iterator
                .enumerate()
                .map(|(bucket, mag)| Bar::new(freq_from_bucket(bucket), mag as f64))
                .collect()
        }

        let buckets = match (phase, full_spectrum) {
            (true, true) => map(spectrum.phases(), |b| spectrum.freq_from_bucket(b)),
            (true, false) => map(spectrum.phases_real(), |b| spectrum.freq_from_bucket(b)),
            (false, true) => map(spectrum.amplitudes(), |b| spectrum.freq_from_bucket(b)),
            (false, false) => map(spectrum.amplitudes_real(), |b| spectrum.freq_from_bucket(b)),
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
        SidePanel::left("left_panel").show(ctx, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
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
                        Slider::new(&mut self.fft_width, 1..=14)
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
                    ui.add(Slider::new(&mut self.hop_frac, 1..=16));

                    let max_cursor = self.waveform.len() - self.window_width - 1;
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

        // Get the slice of the waveform to work on
        let waveform = self.waveform.slice(cursor..(cursor + fft_width));

        let mut spectrum = self
            .analyzer
            .spectrum(&waveform, self.window_width, self.window);

        TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.label(format!(
                "Frequency Resolution: {} Hz",
                spectrum.freq_resolution()
            ));

            ui.label(format!("FFT algorithm: cfft_{}", fft_width));
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
                            .width(2.5)
                            .name("Start of Window"),
                    );
                    ui.vline(
                        VLine::new(self.waveform.time_from_sample(cursor + self.window_width))
                            .color(Color32::DARK_RED)
                            .width(1.5)
                            .name("End of Window"),
                    );
                    ui.vline(
                        VLine::new(
                            self.waveform
                                .time_from_sample(cursor + self.window_width / self.hop_frac),
                        )
                        .color(Color32::GOLD)
                        .name("Start of Next Window"),
                    );
                    ui.vline(
                        VLine::new(
                            self.waveform
                                .time_from_sample(self.playback_head.load(Ordering::SeqCst)),
                        )
                        .color(Color32::LIGHT_BLUE)
                        .name("Playback Head"),
                    );
                });

            Plot::new("window_samples")
                .height(ui.available_height() / 2.0)
                .center_y_axis(true)
                .legend(Legend::default())
                .include_y(1.0)
                .include_y(-1.0)
                .show(ui, |ui| {
                    ui.points(
                        Points::new(Values::from_ys_f32(waveform.samples()))
                            .name("Samples")
                            .stems(0.0),
                    );

                    ui.line(
                        Line::new(Values::from_values_iter(
                            self.window
                                .into_iter(self.window_width)
                                .chain(iter::repeat(0.0))
                                .enumerate()
                                .map(|(i, w)| Value::new(i as f32, w))
                                .take(fft_width),
                        ))
                        .name("Window"),
                    );

                    ui.points(
                        Points::new(Values::from_values_iter(
                            waveform
                                .samples()
                                .iter()
                                .zip(self.window.into_iter(self.window_width))
                                .enumerate()
                                .map(|(i, (sample, w))| Value::new(i as f32, w * sample)),
                        ))
                        .name("Windowed Samples")
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
    }

    fn name(&self) -> &str {
        "Shitty Loid"
    }
}

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
    thread,
    time::Duration,
};

use atomic::Atomic;
use audio::waveform::Waveform;
use eframe::{
    egui::{
        Button, CentralPanel, Context, Layout, ProgressBar, RichText, Slider, TextFormat,
        TopBottomPanel, Ui, Visuals, Window,
    },
    emath::{Align, Align2},
    epaint::{text::LayoutJob, Color32, TextureHandle, Vec2},
    epi::{self, App, Storage, APP_KEY},
};
use once_cell::sync::Lazy;
use parking_lot::{RwLock, RwLockReadGuard};
use ritelinked::LinkedHashSet;
use static_assertions::const_assert;

use crate::{
    analysis::{analyze, AnalysisOptions, KeyPress, KeyPresses},
    decode::AudioDecoder,
    key::{Accidental, PianoKey},
    midi::{MidiPlayer, SongProgress},
    piano_roll::PianoRoll,
    ui_error::UiError,
};

pub struct Application {
    recently_opened_files: LinkedHashSet<PathBuf>,

    // FIXME: fix this abomination
    seconds_per_width: f32,
    key_height: f32,
    preference: Accidental,
    spectrogram: bool,

    // FIXME: RWLock really useful at all?
    waveform: Arc<RwLock<Option<Waveform<'static>>>>,
    analysis: Arc<RwLock<Option<AudioAnalysis>>>,
    analysis_options: AnalysisOptions,
    status: Arc<Atomic<TaskProgress>>,

    midi: MidiPlayer,
    current_song: SongProgress,

    // Error reporting
    previous_error: Option<Box<dyn UiError>>,
}

struct AudioAnalysis {
    notes: BTreeMap<PianoKey, KeyPresses>,
    spectrum: Option<TextureHandle>,
}

#[derive(Debug, Clone, Copy)]
#[repr(align(8))] // Allow native atomic instructions
pub enum TaskProgress {
    None,
    Decoding(f32),
    Analyzing(f32),
    GeneratingSpectrogram,
}

// Ensure that native atomic instructions are being used
const_assert!(Atomic::<TaskProgress>::is_lock_free());

impl Application {
    pub fn new(recently_opened_files: LinkedHashSet<PathBuf>) -> Self {
        let test_pattern = PianoKey::all()
            .enumerate()
            .map(|(index, key)| {
                let duration = Duration::from_secs_f32(0.1);
                let spacing = (0.1 * 1000.0) as u64;

                (
                    key,
                    KeyPresses::from([
                        KeyPress::new(spacing * index as u64, duration, 1.0),
                        KeyPress::new(spacing * 10, duration, 2.0),
                        KeyPress::new(
                            spacing * (PianoKey::all().len() - index) as u64,
                            duration,
                            0.5,
                        ),
                    ]),
                )
            })
            .collect();

        Self {
            previous_error: None,

            recently_opened_files,

            midi: MidiPlayer::new(crate::NAME),
            current_song: SongProgress::new(),

            seconds_per_width: 30.0,
            key_height: 10.0,
            preference: Accidental::Flat,
            spectrogram: true,

            analysis_options: AnalysisOptions {
                threshold: 100.0,
                fft_size: 14,
                window_fraction: 0.5,
                step_fraction: 1.0,
            },
            analysis: Arc::new(RwLock::new(Some(AudioAnalysis {
                notes: test_pattern,
                spectrum: None,
            }))),
            waveform: Default::default(),
            status: Arc::new(Atomic::new(TaskProgress::None)),
        }
    }

    fn open_file(&mut self, path: PathBuf, ctx: Context) {
        if let Err(error) = self.open_file_inner(path, ctx) {
            self.previous_error = Some(error);
        }
    }

    fn open_file_inner(&mut self, path: PathBuf, ctx: Context) -> Result<(), Box<dyn UiError>> {
        let (decoder, path) = AudioDecoder::create_for_file(path)?;

        // Add to recently opened files if decoder created successfully
        self.recently_opened_files.insert(path);

        let status = self.status.clone();
        let waveform = self.waveform.clone();
        let analysis = self.analysis.clone();

        thread::Builder::new()
            .name("file-decode".to_string())
            .spawn(move || {
                status.store(TaskProgress::Decoding(0.0), Ordering::SeqCst);
                ctx.request_repaint();

                let new_waveform = decoder.decode(&|progress| {
                    status.store(TaskProgress::Decoding(progress), Ordering::SeqCst);
                    ctx.request_repaint();
                });

                *waveform.write() = Some(new_waveform);
                *analysis.write() = None;

                status.store(TaskProgress::None, Ordering::SeqCst);
                ctx.request_repaint();
            })
            .expect("unable to spawn decode thread");

        Ok(())
    }

    fn analyze_waveform(&self, ctx: Context) {
        let status = self.status.clone();
        let waveform = self.waveform.clone();
        let analysis = self.analysis.clone();
        let analysis_options = self.analysis_options;

        thread::Builder::new()
            .name("waveform-analysis".to_string())
            .spawn(move || {
                status.store(TaskProgress::Analyzing(0.0), Ordering::SeqCst);
                ctx.request_repaint();

                let waveform = waveform.read();
                let waveform = match waveform.as_ref() {
                    Some(w) => w,
                    None => {
                        tracing::warn!("attempted to analyze a missing waveform");

                        return;
                    }
                };

                let (notes, image) = analyze(waveform, analysis_options, &|progress| {
                    status.store(TaskProgress::Analyzing(progress), Ordering::SeqCst);
                    ctx.request_repaint();
                });

                status.store(TaskProgress::GeneratingSpectrogram, Ordering::SeqCst);
                ctx.request_repaint();
                // FIXME: is the above code useful? the context stays locked the whole time the
                // image is loaded, so unless we inject an artificial delay here the context will
                // stay locked, preventing the repaint of the gui.

                // Ensure the texture uploaded is within the size supported by the graphics driver
                let max_texture_side = ctx.input().max_texture_side;
                let spectrum =
                    if image.width() > max_texture_side || image.height() > max_texture_side {
                        tracing::error!(
                            "{}x{} image has dimensions above the maximum side length of {}",
                            image.width(),
                            image.height(),
                            max_texture_side
                        );

                        None
                    } else {
                        Some(ctx.load_texture("fft-spectrum", image))
                    };

                *analysis.write() = Some(AudioAnalysis { notes, spectrum });

                status.store(TaskProgress::None, Ordering::SeqCst);
                ctx.request_repaint();
            })
            .expect("unable to spawn analysis thread");
    }

    // TODO: make sexier
    fn detect_files_being_dropped(&mut self, ui: &mut Ui) {
        use eframe::egui::*;

        // Preview hovering files:
        if !ui.input().raw.hovered_files.is_empty() {
            let mut text = "Dropping files:\n".to_owned();
            for file in &ui.input().raw.hovered_files {
                if let Some(path) = &file.path {
                    text += &format!("\n{}", path.display());
                } else if !file.mime.is_empty() {
                    text += &format!("\n{}", file.mime);
                } else {
                    text += "\n???";
                }
            }

            let painter = Painter::new(
                ui.ctx().clone(),
                LayerId::new(Order::Foreground, Id::new("file_drop_target")),
                ui.clip_rect(),
            );

            let screen_rect = ui.clip_rect();
            painter.rect_filled(screen_rect, 0.0, Color32::from_black_alpha(192));
            painter.text(
                screen_rect.center(),
                Align2::CENTER_CENTER,
                text,
                TextStyle::Heading.resolve(ui.style()),
                Color32::WHITE,
            );
        }

        // Open dropped files
        // TODO: support multiple and web
        if let Some(dropped_file) = ui.input_mut().raw.dropped_files.pop() {
            self.open_file(
                dropped_file
                    .path
                    .expect("drag and drop not supported on web platform yet"),
                ui.ctx().clone(),
            )
        }
    }
}

impl App for Application {
    fn update(&mut self, ctx: &Context, frame: &mut epi::Frame) {
        if let Some(error) = self.previous_error.take() {
            Window::new("Error")
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .auto_sized()
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width() / 2.0, f32::INFINITY),
                        Layout::top_down(Align::Center),
                        |ui| {
                            error.ui_error(ui);

                            ui.add_space(1.0);

                            if !ui.button("Ok").clicked() {
                                self.previous_error.replace(error);
                            }
                        },
                    );
                });
        }

        {
            let analysis = match self.status.load(Ordering::SeqCst) {
                TaskProgress::None => None,
                TaskProgress::Decoding(progress) => Some(("Decoding", progress)),
                TaskProgress::Analyzing(progress) => Some(("Analyzing", progress)),
                // TODO: Better display for this
                TaskProgress::GeneratingSpectrogram => Some(("Generating Spectrogram", 0.5)),
            };

            if let Some((step, progress)) = analysis {
                Window::new(step)
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .auto_sized()
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.add(
                            ProgressBar::new(progress)
                                .show_percentage()
                                .desired_width(ui.available_width() / 2.0),
                        )
                    });
            }
        }

        TopBottomPanel::top("nav_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open File…").clicked() {
                        ui.close_menu();

                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            self.open_file(path, ctx.clone());
                        }
                    }
                    ui.add_enabled_ui(!self.recently_opened_files.is_empty(), |ui| {
                        ui.menu_button("Open Recent", |ui| {
                            let mut selected_file = None;

                            // Reverse the iterator, bottom == newest
                            for file in self.recently_opened_files.iter().rev() {
                                // TODO: don't panic?
                                let filename = file
                                    .file_name()
                                    .expect("All previous files must have a filename")
                                    .to_string_lossy();
                                let path = file
                                    .parent()
                                    .expect("Files should have a parent directory")
                                    .to_string_lossy();

                                if ui
                                    .add(
                                        Button::new({
                                            let mut job = LayoutJob::default();

                                            let format = TextFormat {
                                                color: Color32::DARK_GRAY,
                                                ..Default::default()
                                            };

                                            job.append(&path, 0.0, format.clone());
                                            job.append(
                                                &std::path::MAIN_SEPARATOR.to_string(),
                                                0.0,
                                                format,
                                            );
                                            job.append(&filename, 0.0, Default::default());

                                            job
                                        })
                                        .wrap(false),
                                    )
                                    .clicked()
                                {
                                    ui.close_menu();

                                    selected_file = Some(file.clone());
                                }
                            }

                            // Delay file open until all files have been put on screen.
                            if let Some(selected_file) = selected_file {
                                self.open_file(selected_file, ctx.clone());
                            }

                            ui.separator();

                            if ui.button("Clear Recently Opened").clicked() {
                                self.recently_opened_files.clear();
                            }
                        });
                    });
                });
                ui.menu_button("View", |ui| {
                    ui.menu_button("Accidental Preference", |ui| {
                        // TODO: add font with the flat+sharp chars
                        ui.selectable_value(&mut self.preference, Accidental::Flat, "Flats (b)");
                        ui.selectable_value(&mut self.preference, Accidental::Sharp, "Sharps (#)");

                        ui.separator();

                        // TODO: Scales
                        ui.add_enabled_ui(false, |ui| ui.menu_button("Scale", |_ui| {}))
                    });

                    ui.menu_button("Theme", |ui| {
                        // eframe::egui::widgets::global_dark_light_mode_buttons(ui)
                        let mut visuals = ui.ctx().style().visuals.clone();

                        ui.selectable_value(&mut visuals, Visuals::light(), "☀ Light");
                        ui.selectable_value(&mut visuals, Visuals::dark(), "🌙 Dark");

                        ui.ctx().set_visuals(visuals);
                    })
                });

                ui.with_layout(Layout::right_to_left(), |ui| {
                    ui.label(" ms");
                    ui.label(
                        RichText::new(format!(
                            "{:.2}",
                            frame.info().cpu_usage.unwrap_or(0.0) * 1000.0
                        ))
                        .monospace(),
                    );
                    ui.label("Last frame: ");
                });
            });
        });

        CentralPanel::default().show(ctx, |ui| {
            let analysis = self.analysis.clone();

            let notes = ui
                .horizontal_wrapped(|ui| {
                    ui.vertical(|ui| {
                        ui.heading("Waveform");
                        ui.set_enabled(self.waveform.read().is_some());

                        if ui.button("Unload").clicked() {
                            *self.waveform.write() = None;
                            *self.analysis.write() = None;
                        }

                        let waveform = self.waveform.read();
                        let waveform = waveform.as_ref();

                        ui.label(format!(
                            "Duration: {:.2}s",
                            waveform.map(|w| w.duration()).unwrap_or(f32::NAN)
                        ));
                        ui.label(format!(
                            "Sample Rate: {} Hz",
                            waveform.map(|w| w.sample_rate()).unwrap_or_default()
                        ));
                        ui.label(format!(
                            "Samples: {}",
                            waveform.map(|w| w.len()).unwrap_or_default()
                        ));
                    });

                    ui.vertical(|ui| {
                        ui.heading("Analysis Options");

                        let waveform = self.waveform.read();
                        ui.set_enabled(waveform.is_some());
                        ui.horizontal(|ui| {
                            ui.add(
                                Slider::new(&mut self.analysis_options.fft_size, 1..=14)
                                    .text("FFT Width")
                                    .prefix("2^")
                                    .suffix(" samples"),
                            );
                            ui.label(format!(
                                "({:.4}s)",
                                waveform
                                    .as_ref()
                                    .map(|w| w.time_from_sample(self.analysis_options.fft_width()))
                                    .unwrap_or_default()
                            ));
                        });

                        ui.horizontal(|ui| {
                            ui.add(
                                Slider::new(&mut self.analysis_options.window_fraction, 0.0..=1.0)
                                    .text("Window Fraction")
                                    .logarithmic(true),
                            );
                            ui.label(format!(
                                "({:.4}s)",
                                waveform
                                    .as_ref()
                                    .map(|w| w
                                        .time_from_sample(self.analysis_options.window_width()))
                                    .unwrap_or_default()
                            ));
                        });

                        ui.horizontal(|ui| {
                            ui.add(
                                Slider::new(&mut self.analysis_options.step_fraction, 0.0..=1.0)
                                    .text("Step Fraction")
                                    .logarithmic(true),
                            );
                            ui.label(format!(
                                "({:.4}s)",
                                waveform
                                    .as_ref()
                                    .map(|w| w.time_from_sample(self.analysis_options.step()))
                                    .unwrap_or_default()
                            ));
                        });

                        ui.add(
                            Slider::new(&mut self.analysis_options.threshold, 0.0..=1000.0)
                                .text("Note threshold"),
                        );

                        drop(waveform);

                        if ui.button("Analyze").clicked() {
                            self.analyze_waveform(ui.ctx().clone());
                        }
                    });

                    let notes = ui
                        .vertical(|ui| {
                            ui.heading("Analysis");

                            ui.set_enabled(analysis.read().is_some());

                            if ui.button("Unload").clicked() {
                                *analysis.write() = None;
                                if let Some(progress) = self.current_song.upgrade() {
                                    progress.cancel();
                                }
                            }

                            let notes = RwLockReadGuard::map(analysis.read(), |analysis| {
                                static EMPTY: Lazy<BTreeMap<PianoKey, KeyPresses>> =
                                    Lazy::new(BTreeMap::new);

                                analysis
                                    .as_ref()
                                    .map(|analysis| &analysis.notes)
                                    .unwrap_or(&EMPTY)
                            });

                            let notes_count = notes
                                .values()
                                .map(|key_presses| key_presses.len())
                                .sum::<usize>();

                            if let Some(progress) = self.current_song.upgrade() {
                                ui.label(format!(
                                    "Played {} of {} notes",
                                    progress.notes_played(),
                                    notes_count
                                ));
                                ui.label(format!("{:.2}s", progress.time()));
                            } else {
                                ui.label(format!("Loaded {} notes", notes_count));
                            }

                            ui.horizontal(|ui| match self.current_song.upgrade() {
                                Some(progress) => {
                                    if ui.button("Stop Playing").clicked() {
                                        progress.cancel();
                                    }
                                }
                                None => {
                                    if ui.button("Play Notes").clicked() {
                                        self.current_song =
                                            self.midi.play_song(&notes, ctx.clone());
                                    }
                                }
                            });

                            notes
                        })
                        .inner;

                    ui.vertical(|ui| {
                        ui.heading("Visualization");
                        ui.checkbox(&mut self.spectrogram, "Show Spectrogram");
                        ui.add(
                            Slider::new(&mut self.seconds_per_width, 1.0..=100.0).text("Scale X"),
                        );
                        ui.add(Slider::new(&mut self.key_height, 1.0..=100.0).text("Scale Y"));
                    });

                    notes
                })
                .inner;

            {
                let analysis = self.analysis.read();
                let spectrum = if self.spectrogram {
                    analysis
                        .as_ref()
                        .and_then(|analysis| analysis.spectrum.as_ref())
                } else {
                    None
                };

                ui.add(PianoRoll::new(
                    &self.midi,
                    self.preference,
                    self.current_song.upgrade().map(|progress| progress.time()),
                    self.key_height,
                    self.seconds_per_width,
                    &notes,
                    spectrum,
                ));
            }

            self.detect_files_being_dropped(ui);
        });
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        epi::set_value(storage, APP_KEY, &self.recently_opened_files);
    }

    fn persist_native_window(&self) -> bool {
        false
    }

    fn persist_egui_memory(&self) -> bool {
        false
    }
}

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
        Button, CentralPanel, Context, Grid, Layout, ProgressBar, ScrollArea, Slider, TextFormat,
        TopBottomPanel, Ui, Visuals, Window,
    },
    emath::{Align, Align2},
    epaint::{text::LayoutJob, Color32, TextureHandle, Vec2},
    epi::{self, App, Storage, APP_KEY},
};
use parking_lot::RwLock;
use ritelinked::LinkedHashSet;
use static_assertions::const_assert;

use crate::{
    analysis::analyze,
    decode::AudioDecoder,
    key::{Accidental, PianoKey},
    midi::{MidiPlayer, SongProgress},
    piano_roll::{KeyPress, KeyPresses, PianoRoll},
    ui_error::UiError,
};

pub struct Application {
    recently_opened_files: LinkedHashSet<PathBuf>,

    // FIXME: fix this abomination
    seconds_per_width: f32,
    key_height: f32,
    preference: Accidental,

    threshold: f32,

    waveform: Arc<RwLock<Option<Waveform<'static>>>>,
    analysis: Arc<RwLock<Option<AudioAnalysis>>>,
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

            threshold: 100.0,

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

                status.store(TaskProgress::None, Ordering::SeqCst);
                ctx.request_repaint();
            })
            .expect("unable to spawn decode thread");

        Ok(())
    }

    fn analyze_waveform(&mut self, ctx: Context) {
        let status = self.status.clone();
        let waveform = self.waveform.clone();
        let analysis = self.analysis.clone();
        let threshold = self.threshold;

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

                let (notes, image) = analyze(waveform, threshold, &|progress| {
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
    fn update(&mut self, ctx: &Context, _frame: &mut epi::Frame) {
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
            });
        });

        if let Some(spectrum) = self
            .analysis
            .read()
            .as_ref()
            .and_then(|analysis| analysis.spectrum.as_ref())
        {
            TopBottomPanel::bottom("spectrum")
                .resizable(true)
                .frame(eframe::egui::Frame::dark_canvas(&ctx.style()))
                .show(ctx, |ui| {
                    ScrollArea::both().show(ui, |ui| ui.image(spectrum, spectrum.size_vec2()));
                });
        }

        CentralPanel::default().show(ctx, |ui| {
            if self.waveform.read().is_some() {
                if ui.button("Analyze").clicked() {
                    self.analyze_waveform(ui.ctx().clone());
                }
            }

            {
                let notes = self.analysis.read();
                let notes = notes.as_ref().map(|analysis| &analysis.notes);

                ui.set_enabled(notes.is_some());

                let empty_tree = BTreeMap::new();

                let notes = notes.unwrap_or(&empty_tree);

                ui.horizontal(|ui| {
                    match self.current_song.upgrade() {
                        Some(progress) => {
                            if ui.button("Stop Playing").clicked() {
                                progress.cancel();
                            }
                        }
                        None => {
                            if ui.button("Play Notes").clicked() {
                                self.current_song = self.midi.play_song(notes);
                            }
                        }
                    }

                    ui.add(Slider::new(&mut self.threshold, 0.0..=1000.0).text("Note threshold"));
                });

                let notes_played = self
                    .current_song
                    .upgrade()
                    .map(|progress| progress.notes_played())
                    .unwrap_or_default();

                // TODO: Count total notes somehow
                ui.label(format!("Played {} of {} notes", notes_played, notes.len()));

                ui.separator();

                ui.add(Slider::new(&mut self.seconds_per_width, 1.0..=100.0).text("Scale X"));

                Grid::new("piano_roll_grid")
                    .num_columns(2)
                    .min_row_height(ui.available_height())
                    .show(ui, |ui| {
                        ui.add(
                            Slider::new(&mut self.key_height, 1.0..=100.0)
                                .vertical()
                                .text("Scale Y"),
                        );

                        ui.add(PianoRoll::new(
                            &self.midi,
                            self.preference,
                            self.key_height,
                            self.seconds_per_width,
                            notes,
                        ));
                    });
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

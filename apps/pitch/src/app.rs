use std::{
    collections::BTreeMap,
    fs::File,
    ops::Deref,
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
        TopBottomPanel, Ui, Window,
    },
    emath::{Align, Align2},
    epaint::{text::LayoutJob, AlphaImage, Color32, TextureHandle, Vec2},
    epi::{self, App, Storage, APP_KEY},
};
use parking_lot::{RwLock, RwLockReadGuard};
use ritelinked::LinkedHashSet;
use spectrum::WaveformSpectrum;

use crate::{
    decode::AudioDecoder,
    key::{Accidental, PianoKey},
    midi::MidiPlayer,
    piano_roll::{KeyDuration, KeyPress, KeyPresses, PianoRoll},
    ui_error::UiError,
};

pub struct Application {
    recently_opened_files: LinkedHashSet<PathBuf>,

    // FIXME: fix this abomination
    seconds_per_width: f32,
    key_height: f32,
    preference: Accidental,

    threshold: f32,

    // TODO: parking lot?
    notes: Arc<RwLock<Option<BTreeMap<PianoKey, KeyPresses>>>>,
    spectrum: Arc<RwLock<Option<TextureHandle>>>,
    audio_analysis: Arc<Atomic<AudioStep>>,

    midi: MidiPlayer,

    // Error reporting
    previous_error: Option<Box<dyn UiError>>,
}

#[derive(Debug, Clone, Copy)]
#[repr(align(8))] // Allow native atomic instructions
pub enum AudioStep {
    None,
    Decoding(f32),
    Analyzing(f32),
    GeneratingSpectrogram,
}

impl Application {
    pub fn new(recently_opened_files: LinkedHashSet<PathBuf>) -> Self {
        assert!(Atomic::<AudioStep>::is_lock_free());

        Self {
            previous_error: None,

            recently_opened_files,

            midi: MidiPlayer::new(crate::NAME),

            seconds_per_width: 30.0,
            key_height: 10.0,
            preference: Accidental::Flat,

            threshold: 100.0,

            notes: Arc::new(RwLock::new(Some(
                PianoKey::all()
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
                    .collect(),
            ))),
            spectrum: Default::default(),
            audio_analysis: Arc::new(Atomic::new(AudioStep::None)),
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

        let audio_analysis = self.audio_analysis.clone();
        let spectrum = self.spectrum.clone();
        let notes = self.notes.clone();

        let preference = self.preference;
        let threshold = self.threshold;

        thread::Builder::new()
            .name("file-decode-analysis".to_string())
            .spawn(move || {
                audio_analysis.store(AudioStep::Decoding(0.0), Ordering::SeqCst);

                let waveform = decoder.decode(&|progress| {
                    audio_analysis.store(AudioStep::Decoding(progress), Ordering::SeqCst);
                    ctx.request_repaint();
                });

                audio_analysis.store(AudioStep::Analyzing(0.0), Ordering::SeqCst);
                ctx.request_repaint();

                const FFT_WIDTH: usize = 8192;
                const WINDOW_WIDTH: usize = FFT_WIDTH / 2;

                let windows = (0..waveform.len() - WINDOW_WIDTH)
                    .step_by(WINDOW_WIDTH)
                    .map(|start| start..start + WINDOW_WIDTH);
                let window_count = dbg!(windows.len());

                let seconds_per_window = WINDOW_WIDTH as f64 / waveform.sample_rate() as f64;

                let mut image = AlphaImage::new([window_count, FFT_WIDTH / 2]);
                let mut keys = BTreeMap::<PianoKey, KeyPresses>::new();

                for (i, window) in windows.enumerate() {
                    audio_analysis.store(
                        AudioStep::Analyzing(i as f32 / image.width() as f32),
                        Ordering::SeqCst,
                    );
                    ctx.request_repaint();

                    let waveform = waveform.slice(window);
                    let spectrum = waveform.spectrum(spectrum::Window::Hann, FFT_WIDTH);

                    let width = image.width();
                    // let mut max = None;
                    for (pixel, (bucket, amplitude)) in image.pixels[i..]
                        .iter_mut()
                        .step_by(width)
                        .zip(spectrum.amplitudes_real().enumerate())
                    {
                        *pixel = (u8::MAX as f32 * amplitude).round() as u8;

                        // let max = max.get_or_insert((bucket, amplitude));
                        // if amplitude > max.1 {
                        //     *max = (bucket, amplitude)
                        // }

                        if amplitude < threshold {
                            continue;
                        }

                        let frequency = spectrum.freq_from_bucket(bucket) as f32;
                        let key = PianoKey::from_concert_pitch(frequency);

                        if let Some(key) = key {
                            keys.entry(key).or_default().add(KeyPress::new(
                                (i as f64 * seconds_per_window * 1000.0).round() as u64,
                                KeyDuration::from_secs_f64(seconds_per_window),
                                amplitude,
                            ));
                        }
                    }

                    // let dominant = max.map(|(bucket, amplitude)| {
                    //     let frequency = spectrum.freq_from_bucket(bucket) as f32;

                    //     (
                    //         PianoKey::from_concert_pitch(frequency),
                    //         frequency,
                    //         amplitude,
                    //     )
                    // });

                    // if let Some((key, frequency, amplitude)) = dominant {
                    //     if let Some(key) = key {
                    //         // TODO: encode confidence
                    //         keys.entry(key).or_default().add(KeyPress::new(
                    //             (i as f64 * seconds_per_window * 1000.0).round() as u64,
                    //             KeyDuration::from_secs_f64(seconds_per_window),
                    //             amplitude,
                    //         ))
                    //     } else {
                    //         // TODO:
                    //         dbg!(frequency, amplitude);
                    //     }
                    // }
                }

                audio_analysis.store(AudioStep::GeneratingSpectrogram, Ordering::SeqCst);
                ctx.request_repaint();
                // FIXME: is the above code useful? the context stays locked the whole time the
                // image is loaded, so unless we inject an artificial delay here the context will
                // stay locked, preventing the repaint of the gui.

                *notes.write() = Some(keys);

                // TODO: Check texture size
                // ctx.input().max_texture_side

                *spectrum.write() = Some(ctx.load_texture("fft-spectrum", image));

                audio_analysis.store(AudioStep::None, Ordering::SeqCst);
                ctx.request_repaint();
            })
            .expect("unable to spawn decode thread");

        Ok(())
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
    fn update(&mut self, ctx: &Context, _frame: &epi::Frame) {
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
            let analysis = match self.audio_analysis.load(Ordering::SeqCst) {
                AudioStep::None => None,
                AudioStep::Decoding(progress) => Some(("Decoding", progress)),
                AudioStep::Analyzing(progress) => Some(("Analyzing", progress)),
                // TODO: Better display for this
                AudioStep::GeneratingSpectrogram => Some(("Generating Spectrogram", 0.5)),
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
                        // TODO: custom
                        eframe::egui::widgets::global_dark_light_mode_buttons(ui);
                    })
                });
            });
        });

        // Don't block if the spectrum is being written
        if let Some(spectrum) = self
            .spectrum
            .try_read()
            .and_then(|lock| RwLockReadGuard::try_map(lock, |option| option.as_ref()).ok())
        {
            TopBottomPanel::bottom("spectrum")
                .resizable(true)
                .frame(eframe::egui::Frame::dark_canvas(&ctx.style()))
                .show(ctx, |ui| {
                    ScrollArea::both()
                        .show(ui, |ui| ui.image(spectrum.deref(), spectrum.size_vec2()));
                });
        }

        CentralPanel::default().show(ctx, |ui| {
            {
                let notes = self
                    .notes
                    .try_read()
                    .and_then(|lock| RwLockReadGuard::try_map(lock, |option| option.as_ref()).ok());

                ui.set_enabled(notes.is_some());

                let btree = BTreeMap::new();

                let notes = if let Some(notes) = notes.as_ref() {
                    notes
                } else {
                    &btree
                };

                ui.horizontal(|ui| {
                    // TODO: prevent clicks while playing
                    if ui.button("Play Notes").clicked() {
                        self.midi.play_song(notes);
                    }
                    ui.add(Slider::new(&mut self.threshold, 0.0..=1000.0).text("Note threshold"));
                });

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

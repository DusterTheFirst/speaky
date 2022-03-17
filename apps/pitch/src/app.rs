use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use atomic::Atomic;
use audio::waveform::Waveform;
use eframe::{
    egui::{
        style::Margin, Button, CentralPanel, Context, DroppedFile, Grid, ProgressBar, ScrollArea,
        Slider, TextFormat, TopBottomPanel, Ui, Window,
    },
    emath::Align2,
    epaint::{text::LayoutJob, AlphaImage, Color32, ColorImage, TextureHandle, Vec2},
    epi::{self, App, Frame, Storage, APP_KEY},
};
use parking_lot::RwLock;
use ritelinked::LinkedHashSet;
use spectrum::WaveformSpectrum;
use symphonia::core::{
    audio::{AudioBuffer, SampleBuffer},
    codecs::{DecoderOptions, CODEC_TYPE_NULL},
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
    units::TimeBase,
};
use tracing::info;

use crate::{
    key::{Accidental, PianoKey},
    midi::MidiPlayer,
    piano_roll::{KeyDuration, PianoRoll},
};

pub struct Application {
    recently_opened_files: LinkedHashSet<PathBuf>,

    // FIXME: fix this abomination
    seconds_per_width: f32,
    key_height: f32,
    preference: Accidental,

    // TODO: parking lot?
    notes: Arc<RwLock<Option<BTreeMap<PianoKey, BTreeSet<KeyDuration>>>>>,
    spectrum: Arc<RwLock<Option<TextureHandle>>>,
    audio_progress: Arc<(Atomic<AudioStep>, Atomic<f32>)>,

    midi: MidiPlayer,
}

#[derive(Debug, Clone, Copy)]
pub enum AudioStep {
    None,
    Decoding,
    Analyzing,
}

impl Application {
    pub fn new(recently_opened_files: LinkedHashSet<PathBuf>) -> Self {
        Self {
            recently_opened_files,

            midi: MidiPlayer::new(crate::NAME),

            seconds_per_width: 30.0,
            key_height: 15.0,
            preference: Accidental::Flat,

            notes: Default::default(),
            spectrum: Default::default(),
            audio_progress: Arc::new((Atomic::new(AudioStep::None), Atomic::new(0.0))),
        }
    }

    fn open_file(&mut self, path: PathBuf, ctx: Context) {
        // Verify file
        // path.extension()
        let file = File::open(&path).expect("Failed to open file");

        let stream = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|os| os.to_str()) {
            hint.with_extension(extension);
        };

        let probe = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .expect("unsupported format");

        let mut format = probe.format;

        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .expect("no supported audio tracks");

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .expect("unsupported codec");

        // Add to recently opened files if supported
        self.recently_opened_files.insert(path);

        let track_id = track.id;
        let track_frames = track.codec_params.n_frames.expect("unknown duration");

        let audio_progress = self.audio_progress.clone();
        let spectrum = self.spectrum.clone();

        let preference = self.preference;

        thread::spawn(move || {
            let (step, progress) = audio_progress.as_ref();

            step.store(AudioStep::Decoding, Ordering::SeqCst);
            progress.store(0.0, Ordering::SeqCst);

            let mut spec = None;
            let mut sample_buf = None;
            let mut samples = Vec::new();

            // The decode loop.
            loop {
                // Get the next packet from the media format.
                let packet = match format.next_packet() {
                    Ok(packet) => packet,
                    // Err(symphonia::core::errors::Error::ResetRequired) => {
                    //     // The track list has been changed. Re-examine it and create a new set of decoders,
                    //     // then restart the decode loop. This is an advanced feature and it is not
                    //     // unreasonable to consider this "the end." As of v0.5.0, the only usage of this is
                    //     // for chained OGG physical streams.
                    //     unimplemented!();
                    // }
                    Err(symphonia::core::errors::Error::IoError(e))
                        if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                    {
                        info!("Reached end of file");
                        break;
                    }
                    Err(err) => {
                        // A unrecoverable error occured, halt decoding.
                        panic!("{}", err);
                    }
                };

                progress.store(packet.ts() as f32 / track_frames as f32, Ordering::SeqCst);
                ctx.request_repaint();

                // Consume any new metadata that has been read since the last packet.
                while !format.metadata().is_latest() {
                    // Pop the old head of the metadata queue.
                    format.metadata().pop();

                    // Consume the new metadata at the head of the metadata queue.
                }

                // If the packet does not belong to the selected track, skip over it.
                if packet.track_id() != track_id {
                    continue;
                }

                // Decode the packet into audio samples.
                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        let spec = spec.get_or_insert(*decoded.spec());

                        let sample_buf = sample_buf.get_or_insert_with(|| {
                            SampleBuffer::<f32>::new(decoded.capacity() as u64, *spec)
                        });

                        sample_buf.copy_planar_ref(decoded);

                        samples.extend_from_slice(
                            &sample_buf.samples()[..sample_buf.len() / spec.channels.count()],
                        );
                    }
                    // Err(symphonia::core::errors::Error::IoError(_)) => {
                    //     // The packet failed to decode due to an IO error, skip the packet.
                    //     continue;
                    // }
                    // Err(symphonia::core::errors::Error::DecodeError(_)) => {
                    //     // The packet failed to decode due to invalid data, skip the packet.
                    //     continue;
                    // }
                    Err(err) => {
                        // An unrecoverable error occured, halt decoding.
                        panic!("{}", err);
                    }
                }
            }

            step.store(AudioStep::Analyzing, Ordering::SeqCst);
            progress.store(0.0, Ordering::SeqCst);
            ctx.request_repaint();

            if let Some(spec) = spec {
                let waveform = Waveform::new(samples, spec.rate);

                assert_eq!(waveform.len() as u64, track_frames);

                const WINDOW_WIDTH: usize = 700;
                const FFT_WIDTH: usize = 2048;

                let windows = (0..waveform.len() - WINDOW_WIDTH)
                    .step_by(WINDOW_WIDTH)
                    .map(|start| start..start + WINDOW_WIDTH);

                let mut image = AlphaImage::new([dbg!(windows.len()), FFT_WIDTH / 2]);

                for (i, window) in windows.enumerate() {
                    progress.store(i as f32 / image.width() as f32, Ordering::SeqCst);
                    ctx.request_repaint();

                    let waveform = waveform.slice(window);
                    let spectrum = waveform.spectrum(spectrum::Window::Hann, FFT_WIDTH);

                    let width = image.width();
                    for (pixel, amplitude) in image.pixels[i..]
                        .iter_mut()
                        .step_by(width)
                        .zip(spectrum.amplitudes_real())
                    {
                        *pixel = (u8::MAX as f32 * amplitude).round() as u8
                    }

                    // let key = spectrum
                    //     .main_frequency()
                    //     .and_then(|(_bucket, f)| PianoKey::from_concert_pitch(f))
                    //     .map(|key| dbg!(key.as_note(preference)));
                }

                *spectrum.write() = Some(ctx.load_texture("fft-spectrum", image));
            }

            step.store(AudioStep::None, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }

    // TODO: make sexier
    fn detect_files_being_dropped(&mut self, ui: &mut Ui, frame: &Frame) {
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
    fn update(&mut self, ctx: &Context, frame: &epi::Frame) {
        match self.audio_progress.0.load(Ordering::SeqCst) {
            AudioStep::None => {}
            step @ (AudioStep::Decoding | AudioStep::Analyzing) => {
                Window::new(format!("{:?}", step))
                    .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                    .auto_sized()
                    .collapsible(false)
                    .show(ctx, |ui| {
                        ui.add(
                            ProgressBar::new(self.audio_progress.1.load(Ordering::SeqCst))
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

        if let Some(spectrum) = self.spectrum.read().as_ref() {
            TopBottomPanel::bottom("spectrum")
                .resizable(true)
                .frame(eframe::egui::Frame::dark_canvas(&ctx.style()))
                .show(ctx, |ui| {
                    ScrollArea::both().show(ui, |ui| ui.image(spectrum, spectrum.size_vec2()));
                });
        }

        CentralPanel::default().show(ctx, |ui| {
            {
                let notes = self.notes.read();

                ui.set_enabled(notes.is_some());

                let btree = BTreeMap::new();

                let notes = if let Some(notes) = notes.as_ref() {
                    notes
                } else {
                    &btree
                };

                // TODO: prevent clicks while playing
                if ui.button("Play Notes").clicked() {
                    self.midi.play_song(notes);
                }

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

            self.detect_files_being_dropped(ui, frame);
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

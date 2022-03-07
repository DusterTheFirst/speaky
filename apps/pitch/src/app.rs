use std::{collections::HashSet, fs::File, path::PathBuf};

use eframe::{
    egui::{Button, CentralPanel, Context, DroppedFile, TextFormat, TopBottomPanel, Ui},
    epaint::{text::LayoutJob, Color32},
    epi::{self, App, Storage, APP_KEY},
};
use ritelinked::LinkedHashSet;
use symphonia::core::{io::MediaSourceStream, probe::Hint};

use crate::{
    key::PianoKey,
    piano_roll::{KeyDuration, PianoRoll},
};

#[derive(Debug, Default)]
pub struct Application {
    recently_opened_files: LinkedHashSet<PathBuf>,
    dropped_files: Vec<DroppedFile>,
}

impl Application {
    fn open_file(&mut self, path: PathBuf) {
        // Verify file
        // path.extension()
        let file = File::open(&path).expect("Failed to open file");

        let stream = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();

        todo!();

        self.recently_opened_files.insert(path);
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

        // Collect dropped files:
        if !ui.input().raw.dropped_files.is_empty() {
            self.dropped_files = ui.input().raw.dropped_files.clone();
        }
    }
}

impl App for Application {
    fn update(&mut self, ctx: &Context, _frame: &epi::Frame) {
        TopBottomPanel::top("nav_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                eframe::egui::widgets::global_dark_light_mode_switch(ui);
                ui.menu_button("File", |ui| {
                    if ui.button("Open File…").clicked() {
                        ui.close_menu();

                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            self.open_file(path);
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
                                self.open_file(selected_file);
                            }

                            ui.separator();

                            if ui.button("Clear Recently Opened").clicked() {
                                self.recently_opened_files.clear();
                            }
                        });
                    });
                })
            });
        });

        CentralPanel::default().show(ctx, |ui| {
            self.detect_files_being_dropped(ui);

            ui.add(PianoRoll::new(
                PianoKey::all()
                    .map(|key| {
                        (
                            key,
                            HashSet::from([KeyDuration {
                                start: 15 * key.key_u8() as u64,
                                duration: 30,
                            }]),
                        )
                    })
                    .collect(),
                15.0,
            ));
        });
    }

    fn setup(&mut self, _ctx: &Context, _frame: &epi::Frame, storage: Option<&dyn Storage>) {
        if let Some(storage) = storage {
            self.recently_opened_files = epi::get_value(storage, APP_KEY).unwrap_or_default();
        }
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        epi::set_value(storage, APP_KEY, &self.recently_opened_files);
    }

    fn name(&self) -> &str {
        "Pitch"
    }

    fn persist_native_window(&self) -> bool {
        false
    }
}

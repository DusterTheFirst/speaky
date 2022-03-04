use std::path::PathBuf;

use eframe::{
    egui::{Button, CentralPanel, Context, TextFormat, TopBottomPanel},
    epaint::{text::LayoutJob, Color32},
    epi::{self, App, Frame, Storage, APP_KEY},
};
use ritelinked::LinkedHashSet;

#[derive(Debug, Default)]
pub struct Application {
    recently_opened_files: LinkedHashSet<PathBuf>,
}

impl Application {
    fn open_file(&mut self, path: PathBuf) {
        self.recently_opened_files.insert(path);
    }
}

impl App for Application {
    fn update(&mut self, ctx: &Context, _frame: &Frame) {
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

        CentralPanel::default().show(ctx, |_ui| {});
    }

    fn name(&self) -> &str {
        "Pitch"
    }

    fn setup(&mut self, _ctx: &Context, _frame: &Frame, storage: Option<&dyn Storage>) {
        if let Some(storage) = storage {
            self.recently_opened_files = epi::get_value(storage, APP_KEY).unwrap_or_default();
        }
    }

    fn save(&mut self, storage: &mut dyn Storage) {
        epi::set_value(storage, APP_KEY, &self.recently_opened_files);
    }
}

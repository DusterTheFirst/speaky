use std::{fs::File, path::PathBuf};

use eframe::{
    egui::{
        Button, CentralPanel, Context, DroppedFile, Frame, Id, ScrollArea, Sense, TextFormat,
        TextStyle, TopBottomPanel, Ui,
    },
    emath::Align2,
    epaint::{text::LayoutJob, Color32, FontId, Pos2, Rect, Rounding, Shape, Stroke, Vec2},
    epi::{self, App, Storage, APP_KEY},
};
use ritelinked::LinkedHashSet;
use symphonia::core::{io::MediaSourceStream, probe::Hint};

use crate::key::{Accidental, PianoKey};

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

            Frame::dark_canvas(ui.style()).show(ui, |ui| {
                let total_available_height = ui.available_height();

                ScrollArea::vertical().show(ui, |ui| {
                    let mut canvas = ui.available_rect_before_wrap();
                    canvas.max.y = f32::INFINITY;

                    let key_height = 15.0;
                    const KEY_COUNT: usize = 88;

                    let left_margin = 20.0;

                    let offset = canvas.min.to_vec2();
                    let margin = Vec2::new(left_margin, 0.0);

                    let mut items = vec![];
                    items.extend((0..KEY_COUNT).flat_map(|key| {
                        let y = key as f32 * key_height;

                        let min = Pos2::new(0.0, y) + offset;

                        [
                            Shape::line_segment(
                                [min + margin, Pos2::new(canvas.width(), y) + offset],
                                Stroke::new(1.0, Color32::WHITE),
                            ),
                            Shape::text(
                                &ui.fonts(),
                                min + Vec2::new(0.0, key_height / 2.0),
                                Align2::LEFT_CENTER,
                                format!("{key:2}"),
                                TextStyle::Monospace.resolve(ui.style()),
                                Color32::WHITE,
                            ),
                        ]
                    }));
                    items.extend((1..=88).map(|key_u8| {
                        let key = PianoKey::new(key_u8).unwrap();

                        let rect = Rect::from_min_size(
                            Pos2::new(15.0 * key_u8 as f32, key_height * key_u8 as f32),
                            Vec2::new(30.0, key_height),
                        )
                        .translate(offset + margin);

                        let response = ui
                            .interact(rect, Id::new(key), Sense::click_and_drag())
                            .on_hover_ui_at_pointer(|ui| {
                                let note = key.as_note(Accidental::Sharp);

                                ui.label({
                                    let mut job = LayoutJob::default();

                                    job.append(
                                        &note.letter().to_string(),
                                        0.0,
                                        TextFormat::simple(FontId::monospace(20.0), Color32::GRAY),
                                    );
                                    if let Some(accidental) = note.accidental() {
                                        job.append(
                                            &accidental.to_string(),
                                            0.0,
                                            TextFormat::simple(
                                                FontId::monospace(20.0),
                                                Color32::GRAY,
                                            ),
                                        );
                                    }
                                    job.append(
                                        &note.octave().to_string(),
                                        0.0,
                                        TextFormat::simple(FontId::monospace(10.0), Color32::GRAY),
                                    );

                                    job
                                });
                            });

                        Shape::rect_filled(
                            rect,
                            Rounding::same(2.0),
                            if response.hovered() {
                                Color32::LIGHT_RED
                            } else {
                                Color32::RED
                            },
                        )
                    }));

                    ui.painter().extend(items);

                    let height = key_height * KEY_COUNT as f32;

                    let mut used_rect = canvas;
                    used_rect.max.y = used_rect.min.y + height.max(total_available_height);

                    ui.allocate_rect(used_rect, Sense::click_and_drag())
                });
            });
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

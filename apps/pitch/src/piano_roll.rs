use std::collections::{HashMap, HashSet};

use eframe::{
    egui::{Frame, Id, Response, ScrollArea, Sense, TextFormat, Ui, Widget},
    emath::Align2,
    epaint::{text::LayoutJob, Color32, FontId, Pos2, Rect, Rounding, Shape, Stroke, Vec2},
};

use crate::key::{Accidental, PianoKey};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub struct KeyDuration {
    // Start in us
    pub start: u64,
    // Duration in us
    pub duration: u64,
}

pub struct PianoRoll {
    keys: HashMap<PianoKey, HashSet<KeyDuration>>,
    key_height: f32,
}

impl PianoRoll {
    // TODO: builder
    pub fn new(keys: HashMap<PianoKey, HashSet<KeyDuration>>, key_height: f32) -> Self {
        Self { keys, key_height }
    }

    fn draw_ui<'s>(
        &'s self,
        ui: &'s Ui,
        drawing_window: Rect,
        left_gutter: f32,
    ) -> impl Iterator<Item = Shape> + 's {
        PianoKey::all().flat_map(move |key| {
            let key_u8 = key.key_u8();

            let y = (key_u8 - 1) as f32 * self.key_height;

            let top_left = Pos2::new(0.0, y) + drawing_window.min.to_vec2();

            [
                Shape::line_segment(
                    [
                        top_left + (Vec2::X * left_gutter),
                        Pos2::new(drawing_window.width(), y) + drawing_window.min.to_vec2(),
                    ],
                    Stroke::new(1.0, Color32::WHITE),
                ),
                // TODO: Measure text and set margin accordingly
                Shape::text(
                    &ui.fonts(),
                    top_left + Vec2::new(0.0, self.key_height / 2.0),
                    Align2::LEFT_CENTER,
                    format!("{key_u8:2}"),
                    FontId::monospace(self.key_height),
                    Color32::WHITE,
                ),
            ]
        })
    }

    fn draw_notes<'s>(
        &'s self,
        ui: &'s Ui,
        drawing_window: Rect,
        left_gutter: f32,
    ) -> impl Iterator<Item = Shape> + 's {
        self.keys.iter().flat_map(move |(key, durations)| {
            let key_u8 = key.key_u8();

            let y = (key_u8 - 1) as f32 * self.key_height;

            durations
                .iter()
                .map(move |&KeyDuration { start, duration }| {
                    // TODO: time scaling
                    let rect = Rect::from_min_size(
                        Pos2::new(start as f32, y),
                        Vec2::new(duration as f32, self.key_height),
                    )
                    .translate(drawing_window.min.to_vec2() + Vec2::X * left_gutter);

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
                                        TextFormat::simple(FontId::monospace(20.0), Color32::GRAY),
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
                })
        })
    }
}

impl Widget for PianoRoll {
    fn ui(self, ui: &mut Ui) -> Response {
        Frame::dark_canvas(ui.style())
            .show(ui, |ui| {
                let total_available_height = ui.available_height();

                ScrollArea::both().show(ui, |ui| {
                    let mut drawing_window = ui.available_rect_before_wrap();
                    drawing_window.max.y = f32::INFINITY;
                    drawing_window.max.x = f32::INFINITY;

                    let left_gutter = 20.0;

                    ui.painter().extend({
                        let mut shapes = Vec::new();

                        shapes.extend(self.draw_ui(ui, drawing_window, left_gutter));
                        shapes.extend(self.draw_notes(ui, drawing_window, left_gutter));

                        shapes
                    });

                    let height = self.key_height * PianoKey::all().len() as f32;

                    ui.allocate_rect(
                        Rect::from_min_size(
                            drawing_window.min,
                            Vec2::new(0.0 /* TODO: Calculate used width */, height.max(total_available_height)),
                        ),
                        Sense::click_and_drag(),
                    )
                });
            })
            .response
    }
}

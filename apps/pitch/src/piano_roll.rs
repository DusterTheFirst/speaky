use std::{
    collections::{BTreeSet, HashMap},
    time::Duration,
};

use eframe::{
    egui::{Frame, Id, Response, ScrollArea, Sense, TextFormat, Ui, Widget},
    emath::Align2,
    epaint::{text::LayoutJob, Color32, FontId, Pos2, Rect, Rounding, Shape, Stroke, Vec2},
};

use crate::{
    key::{Accidental, PianoKey},
    midi::MidiPlayer,
};

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
pub struct KeyDuration {
    // Start in us
    start: u64,
    // Duration in us
    duration: Duration,
}

impl KeyDuration {
    pub fn new(start: u64, duration: Duration) -> Self {
        Self { start, duration }
    }

    pub fn start_secs(&self) -> f32 {
        self.start as f32 / 1000.0
    }

    pub fn duration_secs(&self) -> f32 {
        self.duration.as_secs_f32()
    }

    pub fn end_secs(&self) -> f32 {
        self.start_secs() + self.duration_secs()
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }
}

pub struct PianoRoll<'player> {
    key_height: f32,
    seconds_per_width: f32, // TODO: less jank

    midi: &'player MidiPlayer,

    keys: HashMap<PianoKey, BTreeSet<KeyDuration>>,
}

impl<'player> PianoRoll<'player> {
    // TODO: builder
    pub fn new(
        midi: &'player MidiPlayer,
        key_height: f32,
        seconds_per_width: f32,
        keys: HashMap<PianoKey, BTreeSet<KeyDuration>>,
    ) -> Self {
        Self {
            midi,
            keys,
            seconds_per_width,
            key_height,
        }
    }
}

impl PianoRoll<'_> {
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

            // TODO: hover and click on number

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
                    top_left + Vec2::new(left_gutter, self.key_height / 2.0),
                    Align2::RIGHT_CENTER,
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
        self.keys.iter().flat_map(move |(&key, durations)| {
            let key_u8 = key.key_u8();

            let y = (key_u8 - 1) as f32 * self.key_height;

            durations.iter().map(move |duration| {
                // TODO: time scaling
                let rect = Rect::from_min_size(
                    Pos2::new(duration.start_secs() * self.seconds_per_width, y),
                    Vec2::new(
                        duration.duration_secs() * self.seconds_per_width,
                        self.key_height,
                    ),
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

                if response.clicked() {
                    self.midi.play_piano(key, duration.duration())
                }

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

impl Widget for PianoRoll<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        Frame::dark_canvas(ui.style())
            .show(ui, |ui| {
                ScrollArea::both().show(ui, |ui| {
                    let drawing_window = ui.available_rect_before_wrap();

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
                            Vec2::new(
                                self.keys
                                    .values()
                                    .filter_map(|set| {
                                        set.iter().last().map(|duration| {
                                            duration.end_secs() * self.seconds_per_width
                                        })
                                    })
                                    .reduce(f32::max)
                                    .map(|end| end + left_gutter)
                                    .unwrap_or_default()
                                    .max(drawing_window.width()),
                                height.max(drawing_window.height()),
                            ),
                        ),
                        Sense::click_and_drag(),
                    )
                });
            })
            .response
    }
}

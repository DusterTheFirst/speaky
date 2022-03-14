use std::{
    collections::{BTreeMap, BTreeSet},
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

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Hash)]
pub struct KeyDuration {
    // TODO: better name
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

    pub fn start_micros(&self) -> u64 {
        self.start
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
    // TODO: scales?
    preference: Accidental,

    key_height: f32,
    seconds_per_width: f32, // TODO: less jank

    midi: &'player MidiPlayer,

    keys: BTreeMap<PianoKey, BTreeSet<KeyDuration>>,
}

impl<'player> PianoRoll<'player> {
    // TODO: builder
    pub fn new(
        midi: &'player MidiPlayer,
        preference: Accidental,
        key_height: f32,
        seconds_per_width: f32,
        keys: BTreeMap<PianoKey, BTreeSet<KeyDuration>>,
    ) -> Self {
        Self {
            key_height,
            keys,
            midi,
            preference,
            seconds_per_width,
        }
    }
}

impl PianoRoll<'_> {
    fn draw_key_ui<'s>(
        &'s self,
        ui: &'s Ui,
        drawing_window: Rect,
        margin: Vec2,
        size: Vec2,
    ) -> impl Iterator<Item = Shape> + 's {
        PianoKey::all().flat_map(move |key| {
            let y = (key.key_u8() - 1) as f32 * self.key_height;

            let top_left = Pos2::new(0.0, y) + drawing_window.min.to_vec2() + margin;

            let rect = Rect::from_min_size(top_left, Vec2::new(size.x, self.key_height));

            [
                Shape::rect_filled(
                    rect,
                    Rounding::none(),
                    if key.is_white() {
                        Color32::WHITE.linear_multiply(0.5)
                    } else {
                        Color32::WHITE.linear_multiply(0.05)
                    },
                ),
                // Shape::rect_stroke(rect, Rounding::none(), Stroke::new(1.0, Color32::WHITE)),
                // TODO: Measure text and set margin accordingly
                // TODO: hover and click on number
                Shape::text(
                    &ui.fonts(),
                    top_left + Vec2::new(0.0, self.key_height / 2.0),
                    Align2::RIGHT_CENTER,
                    format!("{:3}", key.as_note(self.preference)),
                    FontId::monospace(self.key_height),
                    if key.is_white() {
                        Color32::WHITE
                    } else {
                        Color32::DARK_GRAY
                    },
                ),
            ]
        })
    }

    fn draw_notes<'s>(
        &'s self,
        ui: &'s Ui,
        drawing_window: Rect,
        margin: Vec2,
    ) -> impl Iterator<Item = Shape> + 's {
        self.keys.iter().flat_map(move |(&key, durations)| {
            let key_u8 = key.key_u8();

            let y = (key_u8 - 1) as f32 * self.key_height;

            durations.iter().flat_map(move |duration| {
                let rect = Rect::from_min_size(
                    Pos2::new(duration.start_secs() * self.seconds_per_width, y),
                    Vec2::new(
                        duration.duration_secs() * self.seconds_per_width,
                        self.key_height,
                    ),
                )
                .translate(drawing_window.min.to_vec2() + margin);

                let response = ui
                    .interact(rect, Id::new((key, duration)), Sense::click_and_drag())
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

                [
                    Shape::rect_filled(
                        rect,
                        Rounding::same(2.0),
                        if response.hovered() {
                            Color32::LIGHT_RED
                        } else {
                            Color32::RED
                        },
                    ),
                    Shape::rect_stroke(rect, Rounding::same(2.0), Stroke::new(2.0, Color32::KHAKI)),
                ]
            })
        })
    }

    fn draw_time_ui<'s>(
        &'s self,
        ui: &'s Ui,
        drawing_window: Rect,
        margin: Vec2,
        size: Vec2,
    ) -> impl Iterator<Item = Shape> + 's {
        (0..((size.x / self.seconds_per_width).floor() as u64)).flat_map(move |second| {
            let x = second as f32 * self.seconds_per_width;

            let offset = margin + drawing_window.min.to_vec2();

            [
                Shape::text(
                    &ui.fonts(),
                    Pos2::new(x, 0.0) + offset,
                    Align2::CENTER_BOTTOM,
                    format!("{second}s"),
                    FontId::monospace(margin.y),
                    Color32::WHITE,
                ),
                Shape::line_segment(
                    [Pos2::new(x, 0.0) + offset, Pos2::new(x, size.y) + offset],
                    Stroke::new(2.0, Color32::BLACK),
                ),
            ]
        })
    }
}

impl Widget for PianoRoll<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        Frame::dark_canvas(ui.style())
            .show(ui, |ui| {
                ScrollArea::both().show(ui, |ui| {
                    let drawing_window = ui.available_rect_before_wrap();

                    let margin = Vec2::new(30.0, 15.0);

                    let size = {
                        let alloc_height = (self.key_height * PianoKey::all().len() as f32)
                            .max(drawing_window.height());

                        let alloc_width = self
                            .keys
                            .values()
                            .filter_map(|set| {
                                set.iter()
                                    .last()
                                    .map(|duration| duration.end_secs() * self.seconds_per_width)
                            })
                            .reduce(f32::max)
                            .unwrap_or_default()
                            .max(drawing_window.width());

                        Vec2::new(alloc_width, alloc_height)
                    };

                    ui.painter().extend({
                        let mut shapes = Vec::new();

                        shapes.extend(self.draw_key_ui(ui, drawing_window, margin, size));
                        shapes.extend(self.draw_time_ui(ui, drawing_window, margin, size));
                        shapes.extend(self.draw_notes(ui, drawing_window, margin));

                        shapes
                    });

                    ui.allocate_rect(
                        Rect::from_min_size(drawing_window.min, size + margin),
                        Sense::click_and_drag(),
                    )
                });
            })
            .response
    }
}

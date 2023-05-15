use std::{collections::BTreeMap, sync::Arc};

use eframe::{
    egui::{Frame, Id, Response, ScrollArea, Sense, TextFormat, Ui, Widget},
    emath::{Align, Align2},
    epaint::{
        text::LayoutJob, Color32, FontId, Fonts, Galley, Pos2, Rect, Rounding, Shape, Stroke,
        TextureHandle, Vec2,
    },
};

use crate::{
    analysis::KeyPresses,
    key::{Accidental, MusicalNote, PianoKey},
    midi::MidiPlayer,
};

pub struct PianoRoll<'player, 'keys, 'spectrum> {
    // TODO: scales?
    preference: Accidental,

    key_height: f32,
    seconds_per_width: f32, // TODO: less jank

    cursor: Option<f32>,

    midi: &'player MidiPlayer,

    keys: &'keys BTreeMap<PianoKey, KeyPresses>,
    spectrum: Option<&'spectrum TextureHandle>,
}

impl<'player, 'keys, 'spectrum> PianoRoll<'player, 'keys, 'spectrum> {
    // TODO: builder
    pub fn new(
        midi: &'player MidiPlayer,
        preference: Accidental,
        cursor: Option<f32>,
        key_height: f32,
        seconds_per_width: f32,
        keys: &'keys BTreeMap<PianoKey, KeyPresses>,
        spectrum: Option<&'spectrum TextureHandle>,
    ) -> Self {
        Self {
            key_height,
            keys,
            midi,
            preference,
            seconds_per_width,
            cursor,
            spectrum,
        }
    }
}

impl PianoRoll<'_, '_, '_> {
    fn layout_key(fonts: &Fonts, note: &MusicalNote, height: f32) -> Arc<Galley> {
        let mut job = LayoutJob::default();

        job.append(
            &note.letter().to_string(),
            0.0,
            TextFormat::simple(FontId::monospace(height), Color32::GRAY),
        );
        let leading_space = if let Some(accidental) = note.accidental() {
            job.append(
                &accidental.to_string(),
                0.0,
                TextFormat {
                    font_id: FontId::monospace(height / 2.0),
                    color: Color32::GRAY,
                    valign: Align::TOP,
                    ..Default::default()
                },
            );

            let width = {
                let mut job = LayoutJob::default();

                job.append(
                    "m",
                    0.0,
                    TextFormat::simple(FontId::monospace(height), Color32::GRAY),
                );

                fonts.layout_job(job).rect.width()
            };

            -width / 2.0
        } else {
            0.0
        };

        job.append(
            &note.octave().to_string(),
            leading_space,
            TextFormat::simple(FontId::monospace(height / 2.0), Color32::GRAY),
        );

        fonts.layout_job(job)
    }

    fn draw_key_text_ui<'s>(
        &'s self,
        ui: &'s Ui,
        top_left: Pos2,
        allocated_space: &'s mut Vec2,
    ) -> impl Iterator<Item = Shape> + 's {
        PianoKey::all().enumerate().map(move |(row, key)| {
            let y = row as f32 * self.key_height;
            // The top left of this key's row
            let top_left = top_left + Vec2::new(0.0, y);

            let note = key.as_note(self.preference);

            let text_galley = Self::layout_key(&ui.fonts(), &note, self.key_height);

            let text_rect = Align2::LEFT_CENTER.anchor_rect(Rect::from_min_size(
                top_left + Vec2::new(0.0, self.key_height / 2.0),
                text_galley.size(),
            ));

            // Update the max width of all of the labels
            *allocated_space =
                allocated_space.max(Vec2::new(text_rect.width(), y + self.key_height));

            // TODO: click play midi note pls thx
            let response = ui
                .interact(text_rect, Id::new(key), Sense::hover())
                .on_hover_ui_at_pointer(|ui| {
                    let note = key.as_note(Accidental::Sharp);

                    let galley = Self::layout_key(&ui.fonts(), &note, 20.0);
                    ui.label(galley);

                    ui.label(format!("Key #{}", key.number()));
                });

            if response.hovered() {
                // TODO: better color and maybe highlight whole key row????
                Shape::galley_with_color(text_rect.min, text_galley, Color32::RED)
            } else {
                Shape::galley(text_rect.min, text_galley)
            }
        })
    }

    fn draw_key_lines_ui(
        &self,
        drawing_window: Rect,
        margin: Vec2,
        size: Vec2,
    ) -> impl Iterator<Item = Shape> + '_ {
        PianoKey::all().enumerate().flat_map(move |(row, key)| {
            let y = row as f32 * self.key_height;

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
                // TODO: make it look better
                Shape::rect_stroke(
                    rect,
                    Rounding::same(0.0),
                    Stroke::new(self.key_height * 0.10, Color32::BLACK),
                ),
            ]
        })
    }

    // TODO: CULLING
    fn draw_notes<'s>(
        &'s self,
        ui: &'s Ui,
        drawing_window: Rect,
        margin: Vec2,
    ) -> impl Iterator<Item = Shape> + 's {
        self.keys.iter().flat_map(move |(&key, key_presses)| {
            let y = (PianoKey::all().len() as u8 - key.number()) as f32 * self.key_height;

            key_presses.iter().flat_map(move |keypress| {
                let rect = Rect::from_min_size(
                    Pos2::new(keypress.start_secs() * self.seconds_per_width, y),
                    Vec2::new(
                        keypress.duration_secs() * self.seconds_per_width,
                        self.key_height,
                    ),
                )
                .translate(drawing_window.min.to_vec2() + margin)
                .shrink2(Vec2::new(0.0, self.key_height * 0.05));

                let response = ui
                    .interact(
                        rect,
                        Id::new((key, keypress.start())),
                        Sense::click_and_drag(),
                    )
                    .on_hover_ui_at_pointer(|ui| {
                        let note = key.as_note(Accidental::Sharp);

                        let galley = Self::layout_key(&ui.fonts(), &note, 20.0);
                        ui.label(galley);

                        ui.label(format!(
                            "Span: {:.2}s-{:.2}s ({:.2}s)",
                            keypress.start_secs(),
                            keypress.end_secs(),
                            keypress.duration_secs()
                        ));
                        ui.label(format!("Intensity: {}", keypress.intensity()));
                    });

                if response.clicked() {
                    self.midi.play_piano(key, keypress.duration())
                }

                [
                    Shape::rect_filled(
                        rect,
                        Rounding::same(2.0),
                        if self.cursor >= Some(keypress.start_secs()) {
                            Color32::GREEN
                        } else if response.hovered() {
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
                // TODO: fix crowding on zoom out
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

    fn draw_cursor(&self, drawing_window: Rect, margin: Vec2, size: Vec2) -> Option<Shape> {
        self.cursor.map(|time| {
            // TODO: extract x and y coords into own function to reduce boilerplate
            let x = time as f32 * self.seconds_per_width;

            let offset = margin + drawing_window.min.to_vec2();

            Shape::line_segment(
                [Pos2::new(x, 0.0) + offset, Pos2::new(x, size.y) + offset],
                Stroke::new(4.0, Color32::GREEN),
            )
        })
    }
}

impl PianoRoll<'_, '_, '_> {}

impl Widget for PianoRoll<'_, '_, '_> {
    fn ui(self, ui: &mut Ui) -> Response {
        Frame::canvas(ui.style())
            .show(ui, |ui| {
                ScrollArea::both().show(ui, |ui| {
                    let drawing_window = ui.available_rect_before_wrap();

                    // let size = {
                    //     let piano_height = self.key_height * PianoKey::all().len() as f32;
                    //     // Fill avaliable space
                    //     let piano_height = piano_height.max(drawing_window.height());

                    //     let alloc_width = self
                    //         .keys
                    //         .values()
                    //         .filter_map(|set| {
                    //             set.last()
                    //                 .map(|keypress| keypress.end_secs() * self.seconds_per_width)
                    //         })
                    //         .reduce(f32::max)
                    //         .unwrap_or_default()
                    //         .max(drawing_window.width());

                    //     Vec2::new(alloc_width, alloc_height)
                    // };

                    let rect = Rect::from_min_size(drawing_window.min, size);

                    let time_text_size = 15.0;

                    let (shapes, margin) = {
                        let mut shapes = Vec::new();

                        let mut left_margin = 0.0;

                        shapes.extend(self.draw_key_text_ui(
                            ui,
                            drawing_window,
                            time_text_size,
                            &mut left_margin,
                        ));

                        // TODO: padding around text relative to text size (em)
                        let margin = Vec2::new(left_margin + 5.0, time_text_size);

                        shapes.extend(self.draw_key_lines_ui(drawing_window, margin, size));
                        shapes.extend(self.draw_time_ui(ui, drawing_window, margin, size));

                        shapes.extend(self.draw_notes(ui, drawing_window, margin));
                        shapes.extend(self.draw_cursor(drawing_window, margin, size));
                        if let Some(spectrum) = self.spectrum {
                            shapes.extend([Shape::image(
                                spectrum.id(),
                                Rect::from_min_size(
                                    drawing_window.min,
                                    spectrum.size_vec2().min(drawing_window.size()),
                                ),
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE.linear_multiply(0.5),
                            )])
                        }

                        (shapes, margin)
                    };

                    ui.painter().extend(shapes);

                    ui.allocate_rect(
                        Rect::from_min_size(drawing_window.min, size + margin),
                        Sense::click_and_drag(),
                    )
                });
            })
            .response
    }
}

struct PianoRollPainter {
    drawing_window: Rect,
    size: Vec2,
}

impl PianoRollPainter {}

// TODO: functions to calculate positions taking into account scaling and all that shit fuck :)

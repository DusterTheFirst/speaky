use std::{collections::BTreeMap, ops::Deref, sync::Arc, time::Duration};

use eframe::{
    egui::{Frame, Id, Response, ScrollArea, Sense, TextFormat, Ui, Widget},
    emath::{Align, Align2},
    epaint::{
        text::LayoutJob, Color32, FontId, Fonts, Galley, Pos2, Rect, Rounding, Shape, Stroke, Vec2,
    },
};
use once_cell::sync::OnceCell;

use crate::{
    key::{Accidental, MusicalNote, PianoKey},
    midi::MidiPlayer,
};

// FIXME: better data representation?
// The start of the keypress in milliseconds
pub type KeyStart = u128;
// The duration of the keypress
pub type KeyDuration = Duration;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct KeyPress {
    pub start: KeyStart,
    info: KeyPressInfo,
}

impl Deref for KeyPress {
    type Target = KeyPressInfo;

    fn deref(&self) -> &Self::Target {
        &self.info
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct KeyPressInfo {
    pub duration: KeyDuration,
    pub intensity: f32,
}

impl KeyPress {
    pub fn new(
        start: impl Into<KeyStart>,
        duration: KeyDuration,
        intensity: impl Into<f32>,
    ) -> Self {
        Self {
            start: start.into(),
            info: KeyPressInfo {
                duration,
                intensity: intensity.into(),
            },
        }
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

#[derive(Debug, Default, Clone)]
pub struct KeyPresses {
    key_list: BTreeMap<KeyStart, KeyPressInfo>,
}

impl FromIterator<KeyPress> for KeyPresses {
    fn from_iter<T: IntoIterator<Item = KeyPress>>(iter: T) -> Self {
        let mut presses = Self::new();
        presses.extend(iter);
        presses
    }
}

impl<const N: usize> From<[KeyPress; N]> for KeyPresses {
    fn from(array: [KeyPress; N]) -> Self {
        let mut presses = Self::new();
        presses.extend(array);
        presses
    }
}

impl Extend<KeyPress> for KeyPresses {
    fn extend<T: IntoIterator<Item = KeyPress>>(&mut self, iter: T) {
        for keypress in iter.into_iter() {
            self.add(keypress);
        }
    }
}

impl KeyPresses {
    pub fn new() -> Self {
        Self {
            key_list: BTreeMap::new(),
        }
    }

    pub fn iter(
        &self,
    ) -> impl ExactSizeIterator<Item = KeyPress> + DoubleEndedIterator<Item = KeyPress> + '_ {
        self.key_list
            .iter()
            .map(|(&start, &info)| KeyPress { start, info })
    }

    pub fn len(&self) -> usize {
        self.key_list.len()
    }

    pub fn first(&self) -> Option<KeyPress> {
        self.iter().next()
    }

    pub fn last(&self) -> Option<KeyPress> {
        self.iter().next_back()
    }

    // FIXME: what do about intensity
    // FIXME: do at analysis time?
    pub fn add(&mut self, mut keypress: KeyPress) {
        // Join with the note before this
        if let Some((
            previous_key_start,
            KeyPressInfo {
                duration: previous_key_duration,
                ..
            },
        )) = self.key_list.range_mut(..keypress.start).next_back()
        {
            // Check if the end of the previous keypress overlaps with the start of this keypress
            if *previous_key_start + previous_key_duration.as_millis() == keypress.start {
                // Extend the previous key's duration
                *previous_key_duration += keypress.duration;

                return;
            }
        }

        // Join with the note after this
        if let Some((
            &next_key_start,
            &KeyPressInfo {
                duration: next_key_duration,
                ..
            },
        )) = self.key_list.range(keypress.start..).next()
        {
            // Check if the end of this keypress overlaps with the start of the next keypress
            if keypress.start + keypress.duration.as_millis() == next_key_start {
                // Extend this key's duration
                keypress.info.duration += next_key_duration;

                // Remove the note after this
                self.key_list.remove(&next_key_start);
            }
        }

        self.key_list.insert(keypress.start, keypress.info);
    }

    // FIXME: Does not verify duration
    pub fn remove(&mut self, keypress: &KeyPress) {
        self.key_list.remove(&keypress.start);
    }
}

pub struct PianoRoll<'player, 'keys> {
    // TODO: scales?
    preference: Accidental,

    key_height: f32,
    seconds_per_width: f32, // TODO: less jank

    cursor: Option<f32>,

    midi: &'player MidiPlayer,

    keys: &'keys BTreeMap<PianoKey, KeyPresses>,
}

impl<'player, 'keys> PianoRoll<'player, 'keys> {
    // TODO: builder
    pub fn new(
        midi: &'player MidiPlayer,
        preference: Accidental,
        cursor: Option<f32>,
        key_height: f32,
        seconds_per_width: f32,
        keys: &'keys BTreeMap<PianoKey, KeyPresses>,
    ) -> Self {
        Self {
            key_height,
            keys,
            midi,
            preference,
            seconds_per_width,
            cursor,
        }
    }
}

impl PianoRoll<'_, '_> {
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

            static WIDTH: OnceCell<f32> = OnceCell::new();
            let width = WIDTH.get_or_init(|| {
                let mut job = LayoutJob::default();

                job.append(
                    "m",
                    0.0,
                    TextFormat::simple(FontId::monospace(height), Color32::GRAY),
                );

                fonts.layout_job(job).rect.width()
            });

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
        drawing_window: Rect,
        top_margin: f32,
        out_left_margin: &'s mut f32,
    ) -> impl Iterator<Item = Shape> + 's {
        PianoKey::all().enumerate().map(move |(row, key)| {
            let y = row as f32 * self.key_height;
            let top_left = Pos2::new(0.0, y + top_margin) + drawing_window.min.to_vec2();

            let note = key.as_note(self.preference);

            let (text, text_rect) = {
                let text_galley = Self::layout_key(&ui.fonts(), &note, self.key_height);

                let text_rect = Align2::LEFT_CENTER.anchor_rect(Rect::from_min_size(
                    top_left + Vec2::new(0.0, self.key_height / 2.0),
                    text_galley.size(),
                ));

                (Shape::galley(text_rect.min, text_galley), text_rect)
            };

            // Update the max left margin
            *out_left_margin = out_left_margin.max(text_rect.width());

            // TODO: hover/click color change somehow also play midi note pls thx
            ui.interact(text_rect, Id::new(key), Sense::hover())
                .on_hover_ui_at_pointer(|ui| {
                    let note = key.as_note(Accidental::Sharp);

                    let galley = Self::layout_key(&ui.fonts(), &note, 20.0);
                    ui.label(galley);

                    ui.label(format!("Key #{}", key.number()));
                });

            // TODO: hover and click on label
            text
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
                        Id::new((key, keypress.start)),
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
                        ui.label(format!("Intensity: {}", keypress.intensity));
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

impl Widget for PianoRoll<'_, '_> {
    fn ui(self, ui: &mut Ui) -> Response {
        Frame::canvas(ui.style())
            .show(ui, |ui| {
                ScrollArea::both().show(ui, |ui| {
                    let drawing_window = ui.available_rect_before_wrap();

                    let size = {
                        let alloc_height = (self.key_height * PianoKey::all().len() as f32)
                            .max(drawing_window.height());

                        let alloc_width = self
                            .keys
                            .values()
                            .filter_map(|set| {
                                set.last()
                                    .map(|keypress| keypress.end_secs() * self.seconds_per_width)
                            })
                            .reduce(f32::max)
                            .unwrap_or_default()
                            .max(drawing_window.width());

                        Vec2::new(alloc_width, alloc_height)
                    };

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

use std::{
    fmt::{self, Display},
    num::NonZeroU8,
};

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub struct MusicalNote {
    letter: NoteLetter,
    accidental: Option<Accidental>,
    octave: u8,
}

// TODO: frequencies (tuning?)
impl Display for MusicalNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.letter)?;

        if let Some(accidental) = self.accidental {
            write!(f, "{accidental}")?;
        }

        write!(f, "{}", self.octave)
    }
}

impl MusicalNote {
    pub fn new(letter: NoteLetter, accidental: impl Into<Option<Accidental>>, octave: u8) -> Self {
        Self {
            letter,
            accidental: accidental.into(),
            octave,
        }
    }

    /// Get the musical note's letter.
    pub fn letter(&self) -> NoteLetter {
        self.letter
    }

    /// Get the musical note's accidental.
    pub fn accidental(&self) -> Option<Accidental> {
        self.accidental
    }

    /// Get the musical note's octave.
    pub fn octave(&self) -> u8 {
        self.octave
    }

    /// Check if two notes represent the same pitch note, even if they
    /// are represented with different letters or accidentals
    pub fn is_same_pitch_as(&self, other: &Self) -> bool {
        self.octave == other.octave && self.semitone_offset() == other.semitone_offset()
    }

    /// Get the amount of semitones this note is off from its octave
    pub fn semitone_offset(&self) -> i8 {
        self.letter.semitone() as i8 + self.accidental.map_or(0, |a| a.semitone_delta())
    }

    /// Get the twelve tone equal temperament semitone from C0
    pub fn semitone(&self) -> u8 {
        // TODO: https://github.com/rust-lang/rust/issues/87840
        ((self.octave * 12) as i8 + self.semitone_offset()) as u8
    }

    pub fn as_key(&self) -> Option<PianoKey> {
        if self.octave > 8 {
            return None;
        }

        // Get semitone from C0
        let semitone = self.semitone();

        // Move it to note from A0 since thats the first key on the piano
        PianoKey::new(semitone.saturating_sub(8))
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Accidental {
    Sharp,
    Flat,
}

impl Accidental {
    /// The semitone change represented by this accidental
    pub fn semitone_delta(&self) -> i8 {
        match self {
            Accidental::Sharp => 1,
            Accidental::Flat => -1,
        }
    }
}

impl Display for Accidental {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match (self, f.alternate()) {
                (Accidental::Sharp, false) => '#',
                (Accidental::Sharp, true) => '♯',
                (Accidental::Flat, false) => 'b',
                (Accidental::Flat, true) => '♭',
            }
        )
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum NoteLetter {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
}

impl NoteLetter {
    /// The semitone in the octave that this note represents
    pub fn semitone(&self) -> u8 {
        // See the table on https://en.wikipedia.org/wiki/Piano_key_frequencies

        match self {
            NoteLetter::C => 0,
            NoteLetter::D => 2,
            NoteLetter::E => 4,
            NoteLetter::F => 5,
            NoteLetter::G => 7,
            NoteLetter::A => 9,
            NoteLetter::B => 11,
        }
    }
}

impl Display for NoteLetter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Just use the debug format for display
        write!(f, "{:?}", self)
    }
}

// An integer piano key in the range 1 - 88
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub struct PianoKey(NonZeroU8);

impl PianoKey {
    /// All the piano keys from highest to lowest
    pub fn all() -> impl DoubleEndedIterator<Item = Self> + ExactSizeIterator<Item = Self> {
        (1..=88).rev().map(|key| PianoKey::new(key).unwrap())
    }

    pub fn new(key: u8) -> Option<Self> {
        match key {
            0 => None,
            1..=88 => NonZeroU8::new(key).map(Self),
            _ => None,
        }
    }

    // TODO: Scales?
    pub fn from_concert_pitch(freq: f32) -> Option<Self> {
        Self::new(((12.0 * (freq / 440.0).log2()).round() as i8 + 49) as u8)
    }

    pub fn concert_pitch(&self) -> f32 {
        let twelfth_root = 2.0f32.powf(1.0 / 12.0);

        // Raise to the power of keys away from A4
        twelfth_root.powi(self.number() as i32 - 49)
    }

    pub fn number(&self) -> u8 {
        self.0.get()
    }

    // TODO: Scales?
    pub fn as_note(&self, preference: Accidental) -> MusicalNote {
        // Although the piano starts with A0, the octave starts with C0
        let key_from_c0 = self.number() + 8;

        // Quantize by the 12 semitones in an octave
        let note_offset = key_from_c0 % 12;
        let octave = key_from_c0 / 12;

        use self::{Accidental::*, NoteLetter::*};

        match (note_offset, preference) {
            (0, _) => MusicalNote::new(C, None, octave),
            (1, Sharp) => MusicalNote::new(C, Sharp, octave),
            (1, Flat) => MusicalNote::new(D, Flat, octave),
            (2, _) => MusicalNote::new(D, None, octave),
            (3, Sharp) => MusicalNote::new(D, Sharp, octave),
            (3, Flat) => MusicalNote::new(E, Flat, octave),
            (4, _) => MusicalNote::new(E, None, octave),
            (5, _) => MusicalNote::new(F, None, octave),
            (6, Sharp) => MusicalNote::new(F, Sharp, octave),
            (6, Flat) => MusicalNote::new(G, Flat, octave),
            (7, _) => MusicalNote::new(G, None, octave),
            (8, Sharp) => MusicalNote::new(G, Sharp, octave),
            (8, Flat) => MusicalNote::new(A, Flat, octave + 1),
            (9, _) => MusicalNote::new(A, None, octave),
            (10, Sharp) => MusicalNote::new(A, Sharp, octave),
            (10, Flat) => MusicalNote::new(B, Flat, octave),
            (11, _) => MusicalNote::new(B, None, octave),
            (12.., _) => unreachable!(),
        }
    }

    pub fn is_white(&self) -> bool {
        // Although the piano starts with A0, the octave starts with C0
        let key_from_c0 = self.number() + 8;

        // Quantize by the 12 semitones in an octave
        let note_offset = key_from_c0 % 12;

        match note_offset {
            0 | 2 | 4 | 5 | 7 | 9 | 11 => true,
            1 | 3 | 6 | 8 | 10 => false,
            12.. => unreachable!(),
        }
    }

    pub fn is_black(&self) -> bool {
        !self.is_white()
    }
}

#[cfg(test)]
mod test {
    use super::{Accidental::*, MusicalNote, NoteLetter::*, PianoKey};

    // TODO: more test cases all around

    #[test]
    fn same_pitch() {
        assert!(MusicalNote::new(A, Sharp, 0).is_same_pitch_as(&MusicalNote::new(B, Flat, 0)))
    }

    #[test]
    fn as_note() {
        // As0
        assert_eq!(
            PianoKey::new(2).unwrap().as_note(Sharp),
            MusicalNote::new(A, Sharp, 0)
        );

        // Bb0
        assert_eq!(
            PianoKey::new(2).unwrap().as_note(Flat),
            MusicalNote::new(B, Flat, 0)
        );

        // C4
        assert_eq!(
            PianoKey::new(40).unwrap().as_note(Sharp),
            MusicalNote::new(C, None, 4)
        );

        // Make sure naturals return the same for both preferences
        assert_eq!(
            PianoKey::new(40).unwrap().as_note(Sharp),
            PianoKey::new(40).unwrap().as_note(Flat),
        );
    }

    #[test]
    fn as_key() {
        assert_eq!(MusicalNote::new(C, None, 4).as_key(), PianoKey::new(40));

        assert_eq!(MusicalNote::new(A, None, 0).as_key(), PianoKey::new(1));

        assert_eq!(MusicalNote::new(A, Sharp, 0).as_key(), PianoKey::new(2));
        assert_eq!(MusicalNote::new(B, Flat, 0).as_key(), PianoKey::new(2));

        assert_eq!(MusicalNote::new(A, None, 1).as_key(), PianoKey::new(13));

        assert_eq!(MusicalNote::new(C, None, 0).as_key(), None);
    }
}

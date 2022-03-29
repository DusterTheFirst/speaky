use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use async_executor::Executor;
use async_io::Timer;
use flume::{Receiver, RecvError, Sender};
use futures_lite::future;
use midir::{MidiOutput, MidiOutputConnection};
use tracing::{debug, info};

use crate::{
    key::PianoKey,
    piano_roll::{KeyPress, KeyPresses},
};

pub struct MidiPlayer {
    sender: Sender<MidiThreadCommand>,
    executor: Arc<Executor<'static>>,
}

pub enum MidiConnection {
    Disconnected { output: MidiOutput },
    Connected { connection: MidiOutputConnection },
}

impl MidiPlayer {
    const CONN_NAME: &'static str = "piano-roll";

    pub fn new(name: &str) -> Self {
        let midi_output = MidiOutput::new(name).expect("unable to enumerate midi devices");

        // TODO: expose and implement selection
        let connection = match midi_output.ports().as_slice() {
            // Connect if there is only one port available
            [port] => {
                let port_name = midi_output.port_name(port).unwrap();

                debug!(%port_name, "Connecting to the only available output port");

                MidiConnection::Connected {
                    connection: midi_output.connect(port, Self::CONN_NAME).unwrap(),
                }
            }
            _ => MidiConnection::Disconnected {
                output: midi_output,
            },
        };

        let (sender, recv) = flume::unbounded();

        let executor = Arc::new(Executor::new());

        thread::Builder::new()
            .name("smol-future-executor".into())
            .spawn({
                let executor = executor.clone();

                move || {
                    future::block_on(executor.run(future::pending::<()>()));
                }
            })
            .unwrap();

        executor.spawn(midi_thread(connection, recv)).detach();

        Self { sender, executor }
    }

    pub fn play_piano(&self, key: PianoKey, duration: Duration) {
        self.sender
            .send(MidiThreadCommand::PlayNote(
                MidiNote::from_piano_key(key),
                duration,
            ))
            .unwrap();
    }

    pub fn play_song(&self, notes: &BTreeMap<PianoKey, KeyPresses>) {
        let song_start = Instant::now();
        let sender = self.sender.clone();

        let mut deadlines = BTreeMap::<Instant, Vec<(PianoKey, KeyPress)>>::new();
        for (key, key_presses) in notes {
            for key_press in key_presses.iter() {
                deadlines
                    .entry(song_start + Duration::from_secs_f32(key_press.start_secs()))
                    .or_default()
                    .push((*key, key_press))
            }
        }

        self.executor
            .spawn(async move {
                while let Some((&deadline, keys)) = deadlines.iter().next() {
                    Timer::at(deadline).await;

                    for (key, key_press) in keys {
                        sender
                            .send(MidiThreadCommand::PlayNote(
                                MidiNote::from_piano_key(*key),
                                key_press.duration(),
                            ))
                            .unwrap();
                    }

                    deadlines.remove(&deadline);
                }
            })
            .detach();
    }
}

async fn midi_thread(mut connection: MidiConnection, thread_commands: Receiver<MidiThreadCommand>) {
    use futures_lite::prelude::*;

    #[derive(Debug)]
    enum MidiAction {
        ChannelClosed,
        NewCommand(MidiThreadCommand),
        NoteOffWake(Instant, HashSet<MidiNote>),
    }

    let mut note_off_deadlines = BTreeMap::<Instant, HashSet<MidiNote>>::new();

    loop {
        let first_deadline = note_off_deadlines.iter().next();
        let deadline_timer = first_deadline
            .map(|(&deadline, notes)| {
                async move {
                    Timer::at(deadline).await;

                    MidiAction::NoteOffWake(deadline, notes.clone())
                }
                .boxed()
            })
            .unwrap_or_else(|| future::pending().boxed());

        // Create future that will accept incoming commands
        let commands_fut = async {
            match thread_commands.recv_async().await {
                Ok(command) => MidiAction::NewCommand(command),
                Err(RecvError::Disconnected) => MidiAction::ChannelClosed,
            }
        };

        // Poll both futures
        match future::or(commands_fut, deadline_timer).await {
            MidiAction::ChannelClosed => return,
            MidiAction::NewCommand(MidiThreadCommand::PlayNote(note, duration)) => {
                match &mut connection {
                    MidiConnection::Disconnected { .. } => {
                        info!(?note, ?duration, "Midi disconnected.. ignoring note");
                    }
                    MidiConnection::Connected { connection } => {
                        connection
                            .send(MidiCommand::NoteOn(note, 0b01111111).to_bytes().as_slice())
                            .unwrap();

                        let deadline = Instant::now() + duration;

                        // Add the key to the deadlines
                        note_off_deadlines.entry(deadline).or_default().insert(note);

                        // Remove any previous deadlines
                        if let Some((&instant, _)) = note_off_deadlines
                            .range(..deadline)
                            .find(|(_, notes)| notes.contains(&note))
                        {
                            note_off_deadlines.entry(instant).or_default().remove(&note);
                        }
                    }
                }
            }
            MidiAction::NoteOffWake(deadline, notes) => match &mut connection {
                MidiConnection::Disconnected { .. } => {
                    info!(?notes, "Midi disconnected.. ignoring note off");
                }
                MidiConnection::Connected { connection } => {
                    note_off_deadlines.remove(&deadline);

                    for note in notes {
                        connection
                            .send(MidiCommand::NoteOff(note, 0b01111111).to_bytes().as_slice())
                            .unwrap();
                    }
                }
            },
        }
    }
}

#[derive(Debug)]
pub enum MidiThreadCommand {
    PlayNote(MidiNote, Duration),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MidiCommand {
    NoteOn(MidiNote, u8),  // 7 bit velocity
    NoteOff(MidiNote, u8), // 7 bit velocity
    AllSoundOff,
    PitchBendChange(u16), // 14 bit
}

impl MidiCommand {
    pub fn to_bytes(self) -> [u8; 3] {
        #[allow(clippy::unusual_byte_groupings)]
        match self {
            MidiCommand::NoteOn(note, velocity) => [0b1001_0000, note.as_u8(), velocity],
            MidiCommand::NoteOff(note, velocity) => [0b1000_0000, note.as_u8(), velocity],
            MidiCommand::AllSoundOff => [0b1011_0000, 0b0_111_1000, 0b0_000_0000],
            MidiCommand::PitchBendChange(change) => [
                0b1110_0000,
                0b01111111 & (change as u8),        // 7 LSB
                0b01111111 & ((change >> 7) as u8), // 7 MSB
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MidiNote(u8);

impl MidiNote {
    pub fn new(number: u8) -> Self {
        assert_eq!(number >> 7, 0, "midi notes can only be between 0-127");

        Self(number)
    }

    pub fn from_piano_key(key: PianoKey) -> Self {
        Self::new(key.number() + 20)
    }

    pub const fn as_u8(&self) -> u8 {
        self.0
    }
}

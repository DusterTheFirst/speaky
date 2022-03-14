use std::{
    collections::{BTreeMap, BTreeSet},
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

use crate::{key::PianoKey, piano_roll::KeyDuration};

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

    pub fn play_song(&self, notes: &BTreeMap<PianoKey, BTreeSet<KeyDuration>>) {
        use futures_lite::prelude::*;

        let start = Instant::now();
        let sender = self.sender.clone();
        let mut notes = notes.clone();

        self.executor
            .spawn(async move {
                loop {
                    let timers = notes
                        .iter()
                        .flat_map(|(key, durations)| {
                            durations.iter().map(move |duration| {
                                async move {
                                    Timer::at(
                                        start + Duration::from_millis(duration.start_micros()),
                                    )
                                    .await;

                                    Some((key, duration))
                                }
                                .boxed()
                            })
                        }) // TODO: Maybe there is a better way than chaining or futures
                        .reduce(|future_1, future_2| future::or(future_1, future_2).boxed())
                        .unwrap_or_else(|| future::ready(None).boxed());

                    // TODO: merge and send keys together if they have the same deadline
                    match timers.await {
                        Some((&key, &duration)) => {
                            sender
                                .send(MidiThreadCommand::PlayNote(
                                    MidiNote::from_piano_key(key),
                                    duration.duration(),
                                ))
                                .unwrap();

                            notes.entry(key).or_default().remove(&duration);
                        }
                        None => break,
                    }
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
        TimerWake(MidiNote),
    }

    let mut note_off = BTreeMap::<MidiNote, Instant>::new();

    loop {
        // Create future that will poll all timers
        let notes_off_fut = note_off
            .iter()
            .map(|(&note, &deadline)| {
                async move {
                    Timer::at(deadline).await;

                    MidiAction::TimerWake(note)
                }
                .boxed()
            })
            // TODO: Maybe there is a better way than chaining or futures
            .reduce(|future_1, future_2| future::or(future_1, future_2).boxed())
            .unwrap_or_else(|| future::pending().boxed());

        // Create future that will accept incoming commands
        let commands_fut = async {
            match thread_commands.recv_async().await {
                Ok(command) => MidiAction::NewCommand(command),
                Err(RecvError::Disconnected) => MidiAction::ChannelClosed,
            }
        };

        // Poll both futures
        match future::or(commands_fut, notes_off_fut).await {
            MidiAction::ChannelClosed => return,
            MidiAction::NewCommand(MidiThreadCommand::PlayNote(note, duration)) => {
                match &mut connection {
                    MidiConnection::Disconnected { .. } => {
                        info!(?note, ?duration, "Midi disconnected.. ignoring note");
                    }
                    MidiConnection::Connected { connection } => {
                        connection
                            .send(MidiCommand::NoteOn(note).to_bytes().as_slice())
                            .unwrap();

                        let deadline = Instant::now() + duration;

                        // Queue command
                        let pre_instant = note_off.entry(note).or_insert(deadline);

                        *pre_instant = deadline.max(*pre_instant);
                    }
                }
            }
            MidiAction::TimerWake(note) => match &mut connection {
                MidiConnection::Disconnected { .. } => {
                    info!(?note, "Midi disconnected.. ignoring note off");
                }
                MidiConnection::Connected { connection } => {
                    note_off.remove(&note);

                    connection
                        .send(MidiCommand::NoteOff(note).to_bytes().as_slice())
                        .unwrap();
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
    NoteOn(MidiNote),
    NoteOff(MidiNote),
}

impl MidiCommand {
    pub fn to_bytes(self) -> Vec<u8> {
        const NOTE_ON_MSG: u8 = 0x90;
        const NOTE_OFF_MSG: u8 = 0x80;
        const VELOCITY: u8 = 0x64;

        match self {
            MidiCommand::NoteOn(note) => vec![NOTE_ON_MSG, note.as_u8(), VELOCITY],
            MidiCommand::NoteOff(note) => vec![NOTE_OFF_MSG, note.as_u8(), VELOCITY],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MidiNote(u8);

impl MidiNote {
    pub fn from_piano_key(key: PianoKey) -> Self {
        Self(key.key_u8() + 20)
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

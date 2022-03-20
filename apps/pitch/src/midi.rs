use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

use async_executor::Executor;
use async_io::Timer;
use flume::{Receiver, RecvError, Sender};
use futures_lite::{future, Future};
use midir::{MidiOutput, MidiOutputConnection};
use tracing::{debug, info};

use crate::{key::PianoKey, piano_roll::KeyPresses};

pub struct MidiPlayer {
    sender: Sender<MidiThreadCommand>,
    executor: Arc<Executor<'static>>,
}

pub enum MidiConnection {
    Disconnected { output: MidiOutput },
    Connected { connection: MidiOutputConnection },
}

/// Future that will poll a [`Vec`] of futures in order
struct Race<O> {
    futures: Vec<Pin<Box<dyn Future<Output = O> + Send>>>,
}

impl<O> Race<O> {
    pub fn new(futures: impl IntoIterator<Item = Pin<Box<dyn Future<Output = O> + Send>>>) -> Self {
        Self {
            futures: futures.into_iter().collect(),
        }
    }
}

impl<O> Future for Race<O> {
    type Output = O;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        for future in &mut self.futures {
            match Future::poll(future.as_mut(), cx) {
                Poll::Ready(val) => return Poll::Ready(val),
                Poll::Pending => continue,
            }
        }

        Poll::Pending
    }
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

    pub fn play_song(&self, notes: &BTreeMap<PianoKey, KeyPresses>) {
        use futures_lite::prelude::*;

        let start = Instant::now();
        let sender = self.sender.clone();
        let mut notes = notes.clone();

        self.executor
            .spawn(async move {
                loop {
                    let timers = Race::new(notes.iter().flat_map(|(&key, key_presses)| {
                        key_presses.iter().map(move |keypress| {
                            async move {
                                Timer::at(start + Duration::from_secs_f32(keypress.start_secs()))
                                    .await;

                                Some((key, keypress))
                            }
                            .boxed()
                        })
                    }));

                    // TODO: merge and send keys together if they have the same deadline
                    match timers.await {
                        Some((key, duration)) => {
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
        let notes_off_fut = Race::new(note_off.iter().map(|(&note, &deadline)| {
            async move {
                Timer::at(deadline).await;

                MidiAction::TimerWake(note)
            }
            .boxed()
        }));

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
        Self(key.number() + 20)
    }

    pub fn as_u8(&self) -> u8 {
        self.0
    }
}

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

        // TODO: expose and implement selection
        let mut connection = match midi_output.ports().as_slice() {
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

        // FIXME: TEMP
        {
            if let MidiConnection::Connected { connection } = &mut connection {
                connection
                    .send(&MidiCommand::AllSoundOff.to_bytes())
                    .unwrap();
                thread::sleep(Duration::from_secs_f32(0.1));
                connection
                    .send(
                        &MidiCommand::NoteOn(
                            MidiNote::from_piano_key(PianoKey::from_concert_pitch(440.0).unwrap()),
                            120,
                        )
                        .to_bytes(),
                    )
                    .unwrap();
                thread::sleep(Duration::from_secs_f32(0.5));
                connection
                    .send(&MidiCommand::PitchBendChange(0x2000 + 100).to_bytes())
                    .unwrap();
            }
        }

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
                            .send(MidiCommand::NoteOn(note, 0b01111111).to_bytes().as_slice())
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
                        .send(MidiCommand::NoteOff(note, 0b01111111).to_bytes().as_slice())
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

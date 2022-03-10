use std::{thread, time::Duration};

use flume::{Receiver, Sender};
use midir::{MidiOutput, MidiOutputConnection};
use tokio::time::sleep;
use tracing::debug;

use crate::key::PianoKey;

pub struct MidiPlayer {
    sender: Sender<MidiThreadCommand>,
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

        thread::spawn(move || midi_thread(connection, recv));

        Self { sender }
    }

    pub fn play_piano(&self, key: PianoKey, duration: Duration) {
        self.sender
            .send(MidiThreadCommand::PlayNote(
                MidiNote::from_piano_key(key),
                duration,
            ))
            .unwrap();
    }
}

fn midi_thread(mut connection: MidiConnection, thread_commands: Receiver<MidiThreadCommand>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();

    let (midi_commands_send, midi_commands_recv) = flume::unbounded::<MidiCommand>();

    thread::spawn(move || {
        for midi_command in midi_commands_recv.iter() {
            if let MidiConnection::Connected { connection } = &mut connection {
                connection.send(&midi_command.to_bytes()).unwrap();
            } else {
                debug!(?midi_command, "dropping midi command");
            }
        }
    });

    runtime.block_on(async move {
        while let Ok(command) = thread_commands.recv_async().await {
            match command {
                MidiThreadCommand::PlayNote(note, duration) => {
                    midi_commands_send
                        .send_async(MidiCommand::NoteOn(note))
                        .await
                        .unwrap();

                    tokio::spawn({
                        let midi_commands_send = midi_commands_send.clone();

                        // FIXME: noteoff can end later notes early
                        // also like this is a mess :(
                        // Do i even need to use futures?
                        async move {
                            sleep(duration).await;
                            midi_commands_send
                                .send_async(MidiCommand::NoteOff(note))
                                .await
                                .unwrap();
                        }
                    });
                }
            }
        }
    });
}

#[derive(Debug)]
pub enum MidiThreadCommand {
    PlayNote(MidiNote, Duration),
}

#[derive(Debug, Clone, Copy)]
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

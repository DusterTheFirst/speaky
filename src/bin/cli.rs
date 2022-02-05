use rodio::{OutputStream, Sink};
use speaky::{load_language, setup_tts, synthesize};
use std::io::{self, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let resources = loop {
        write!(stdout, "language> ").expect("unable to write to stdout");
        stdout.flush().expect("unable to write to stdout");

        let mut lang = String::new();
        stdin
            .read_line(&mut lang)
            .expect("unable to read input line");

        let lang = lang.trim_end();

        match load_language(lang) {
            Ok(resources) => break resources,
            Err(error) => eprintln!("{}", error),
        }
    };

    let mut engine = setup_tts(resources);

    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();

    loop {
        write!(stdout, "synth> ").expect("unable to write to stdout");
        stdout.flush().expect("unable to write to stdout");

        let mut line = String::new();
        stdin
            .read_line(&mut line)
            .expect("unable to read input line");

        let line = line.trim_end();

        sink.append(synthesize(&mut engine, line));

        sink.sleep_until_end();
    }
}

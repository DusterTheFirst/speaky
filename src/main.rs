//! Uses Pico TTS to speak a phrase (via [`cpal`]).

// The MIT License
//
// Copyright (c) 2019 Paolo Jovon <paolo.jovon@gmail.com>
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

use rodio::{buffer::SamplesBuffer, OutputStream, Source};
use std::{io::{self, Write}, rc::Rc, thread};
use ttspico as pico;

fn main() {
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();

    let mut engine = setup_tts();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        write!(stdout, "synth> ").expect("unable to write to stdout");
        stdout.flush().expect("unable to write to stdout");

        let mut line = String::new();
        stdin
            .read_line(&mut line)
            .expect("unable to read input line");

        let speech = synthesize(&mut engine, &line);

        let duration = speech
            .total_duration()
            .expect("speech sample has an unknown duration");

        stream_handle.play_raw(speech.convert_samples()).unwrap();

        thread::sleep(duration);
    }
}

fn synthesize(engine: &mut pico::Engine, text: &str) -> SamplesBuffer<i16> {
    // 5. Put (UTF-8) text to be spoken into the engine
    // See `Engine::put_text()` for more details.
    let text_bytes = text.bytes().chain(std::iter::once(0)).collect::<Vec<_>>();

    let mut text_bytes = &text_bytes[..];
    while !text_bytes.is_empty() {
        let n_put = engine
            .put_text(text_bytes)
            .expect("pico_putTextUtf8 failed");

        text_bytes = &text_bytes[n_put..];
    }

    // 6. Do the actual text-to-speech, getting audio data (16-bit signed PCM @ 16kHz) from the input text
    // Speech audio is computed in small chunks, one "step" at a time; see `Engine::get_data()` for more details.
    let mut pcm_data = Vec::new();
    let mut pcm_buf = [0i16; 1024];
    'tts: loop {
        let (n_written, status) = engine
            .get_data(&mut pcm_buf[..])
            .expect("pico_getData error");

        pcm_data.extend(&pcm_buf[..n_written]);

        if status == ttspico::EngineStatus::Idle {
            break 'tts;
        }
    }

    SamplesBuffer::new(1, 16_000, pcm_data)
}

fn setup_tts() -> pico::Engine {
    // 1. Create a Pico system
    // NOTE: There should at most one System per thread!
    let sys = pico::System::new(4 * 1024 * 1024).expect("Could not init system");

    // 2. Load Text Analysis (TA) and Speech Generation (SG) resources for the voice you want to use
    let ta_res = pico::System::load_resource(Rc::clone(&sys), "lang/en-US_ta.bin")
        .expect("Failed to load TA");
    let sg_res = pico::System::load_resource(Rc::clone(&sys), "lang/en-US_lh0_sg.bin")
        .expect("Failed to load SG");
    println!(
        "TA: {}, SG: {}",
        ta_res.borrow().name().unwrap(),
        sg_res.borrow().name().unwrap()
    );

    // 3. Create a Pico voice definition and attach the loaded resources to it
    let voice = pico::System::create_voice(sys, "TestVoice").expect("Failed to create voice");
    voice
        .borrow_mut()
        .add_resource(ta_res)
        .expect("Failed to add TA to voice");
    voice
        .borrow_mut()
        .add_resource(sg_res)
        .expect("Failed to add SG to voice");

    // 4. Create an engine from the voice definition
    // UNSAFE: Creating an engine without attaching the resources will result in a crash!
    unsafe { pico::Voice::create_engine(voice).expect("Failed to create engine") }
}

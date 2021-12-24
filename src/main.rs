use rodio::{buffer::SamplesBuffer, OutputStream, Source};
use std::{
    io::{self, Write},
    path::{Path},
    rc::Rc,
    thread,
};
use ttspico::{Engine, EngineStatus, System, Voice};

fn main() {
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();

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

    loop {
        write!(stdout, "synth> ").expect("unable to write to stdout");
        stdout.flush().expect("unable to write to stdout");

        let mut line = String::new();
        stdin
            .read_line(&mut line)
            .expect("unable to read input line");

        let line = line.trim_end();

        let speech = synthesize(&mut engine, line);

        let duration = speech
            .total_duration()
            .expect("speech sample has an unknown duration");

        stream_handle.play_raw(speech.convert_samples()).unwrap();

        thread::sleep(duration);
    }
}

fn synthesize(engine: &mut Engine, text: &str) -> SamplesBuffer<i16> {
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
    loop {
        let (n_written, status) = engine
            .get_data(&mut pcm_buf[..])
            .expect("failed to get pico pcm data");

        pcm_data.extend(&pcm_buf[..n_written]);

        if status == EngineStatus::Idle {
            break;
        }
    }

    SamplesBuffer::new(1, 16_000, pcm_data)
}

fn load_language(lang: &str) -> Result<TTSResources, String> {
    let lang_dir = Path::new("./lang");

    if !lang_dir.exists() {
        return Err("languages directory does not exist".to_string());
    }

    let lang = Path::new(lang);

    if lang.components().count() > 1 {
        return Err("language name contains invalid characters".to_string());
    }

    let lang_dir = lang_dir.join(lang);

    if !lang_dir.exists() {
        return Err(format!("{:?} language directory does not exist", lang));
    }

    let text_analysis = lang_dir.join("ta.bin");
    if !text_analysis.exists() {
        return Err(format!(
            "text analysis file does not exist for language {:?}",
            lang
        ));
    }

    let speech_generation = lang_dir.join("sg.bin");
    if !speech_generation.exists() {
        return Err(format!(
            "speech generation file does not exist for language {:?}",
            lang
        ));
    }

    Ok(TTSResources {
        text_analysis: text_analysis.to_str().map(str::to_string).ok_or_else(|| {
            "text analysis file path contained non-unicode characters".to_string()
        })?,
        speech_generation: speech_generation
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| {
                "speech generation file path contained non-unicode characters".to_string()
            })?,
    })
}

struct TTSResources {
    text_analysis: String,
    speech_generation: String,
}

fn setup_tts(
    TTSResources {
        text_analysis,
        speech_generation,
    }: TTSResources,
) -> Engine {
    // 1. Create a Pico system
    // NOTE: There should at most one System per thread!
    let sys = System::new(4 * 1024 * 1024).expect("Could not init ttspico system");

    // 2. Load Text Analysis (TA) and Speech Generation (SG) resources for the voice you want to use
    let ta_res = System::load_resource(Rc::clone(&sys), text_analysis).expect("Failed to load TA");
    let sg_res =
        System::load_resource(Rc::clone(&sys), speech_generation).expect("Failed to load SG");
    println!(
        "TA: {}, SG: {}",
        ta_res.borrow().name().unwrap(),
        sg_res.borrow().name().unwrap()
    );

    // 3. Create a Pico voice definition and attach the loaded resources to it
    let voice = System::create_voice(sys, "TestVoice").expect("Failed to create voice");
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
    unsafe { Voice::create_engine(voice).expect("Failed to create engine") }

    // TODO: make PR on ttspico to make this an impossible situation
}

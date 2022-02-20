use color_eyre::eyre::eyre;
use rodio::buffer::SamplesBuffer;
use std::{path::Path, rc::Rc};
use tracing::info;
use ttspico::{Engine, EngineStatus, System, Voice};

// TODO: better API

#[tracing::instrument(skip_all)]
pub fn setup_tts(
    TTSResources {
        text_analysis,
        speech_generation,
    }: TTSResources,
) -> color_eyre::Result<Engine> {
    // 1. Create a Pico system
    // NOTE: There should at most one System per thread!
    let sys = System::new(4 * 1024 * 1024)
        .map_err(|err| eyre!("could not init ttspico system: {err}"))?;

    // 2. Load Text Analysis (TA) and Speech Generation (SG) resources for the voice you want to use
    let ta_res = System::load_resource(Rc::clone(&sys), text_analysis)
        .map_err(|err| eyre!("failed to load text analysis file: {err}"))?;
    let sg_res = System::load_resource(Rc::clone(&sys), speech_generation)
        .map_err(|err| eyre!("Failed to load speech generation file: {err}"))?;

    info!(
        text_analysis = ta_res.borrow().name().unwrap_or("?"),
        speech_generation = sg_res.borrow().name().unwrap_or("?"),
        "loaded resources",
    );

    // 3. Create a Pico voice definition and attach the loaded resources to it
    let voice = System::create_voice(sys, "TestVoice")
        .map_err(|err| eyre!("failed to create voice: {err}"))?;
    voice
        .borrow_mut()
        .add_resource(ta_res)
        .map_err(|err| eyre!("failed to add text analysis resource to voice: {err}"))?;
    voice
        .borrow_mut()
        .add_resource(sg_res)
        .map_err(|err| eyre!("failed to add speech generation resource to voice: {err}"))?;

    // 4. Create an engine from the voice definition
    // TODO: make PR on ttspico to make this an impossible situation?
    // UNSAFE: Creating an engine without attaching the resources will result in a crash!
    unsafe { Voice::create_engine(voice) }.map_err(|err| eyre!("failed to create engine: {err}"))
}

#[derive(Debug)]
pub struct TTSResources {
    text_analysis: String,
    speech_generation: String,
}

#[tracing::instrument]
pub fn load_language(lang: &str) -> Result<TTSResources, String> {
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

#[tracing::instrument(skip(engine))]
pub fn synthesize(engine: &mut Engine, text: &str) -> color_eyre::Result<SamplesBuffer<i16>> {
    // 5. Put (UTF-8) text to be spoken into the engine
    // See `Engine::put_text()` for more details.
    let mut text_bytes = text.as_bytes();
    while !text_bytes.is_empty() {
        let bytes_put = engine
            .put_text(text_bytes)
            .map_err(|err| eyre!("unable to put text into engine: {err}"))?;

        text_bytes = &text_bytes[bytes_put..];
    }

    engine
        .flush()
        .map_err(|err| eyre!("unable to flush engine: {err}"))?;

    // 6. Do the actual text-to-speech, getting audio data (16-bit signed PCM @ 16kHz) from the input text
    // Speech audio is computed in small chunks, one "step" at a time; see `Engine::get_data()` for more details.
    let mut pcm_data = Vec::new();
    let mut pcm_buf = [0i16; 1024];
    loop {
        let (n_written, status) = engine
            .get_data(&mut pcm_buf[..])
            .map_err(|err| eyre!("failed to get pico pcm data: {err}"))?;

        pcm_data.extend(&pcm_buf[..n_written]);

        if status == EngineStatus::Idle {
            break;
        }
    }

    Ok(SamplesBuffer::new(1, 16_000, pcm_data))
}

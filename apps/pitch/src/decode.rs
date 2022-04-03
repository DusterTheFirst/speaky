use std::{fs::File, io, path::PathBuf};

use audio::waveform::Waveform;
use eframe::{
    egui::{Grid, RichText, Ui},
    epaint::Color32,
};
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL},
    formats::{FormatOptions, FormatReader},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};
use tracing::info;

use crate::ui_error::UiError;

#[derive(Debug)]
pub enum CreateDecoderError {
    OpenFile(PathBuf, io::Error),
    UnsupportedAudioFormat,
    NoSupportedAudioTrack,
    UnknownDuration,
    UnknownCodec,
}

impl From<CreateDecoderError> for Box<dyn UiError> {
    fn from(error: CreateDecoderError) -> Self {
        Box::new(error) as _
    }
}

impl UiError for CreateDecoderError {
    fn ui_error(&self, ui: &mut Ui) {
        match self {
            CreateDecoderError::OpenFile(path, io_error) => {
                ui.label(
                    RichText::new("Unable to open file for decoding")
                        .heading()
                        .color(Color32::RED),
                );

                Grid::new("create_decoder_error")
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label("file:");
                        ui.label(path.display().to_string());
                        ui.end_row();
                        ui.label("error:");
                        ui.label(io_error.to_string());
                    });
            }
            CreateDecoderError::UnsupportedAudioFormat => {
                ui.label(
                    RichText::new("Unsupported audio format")
                        .heading()
                        .color(Color32::RED),
                );
            }
            CreateDecoderError::NoSupportedAudioTrack => {
                ui.label(
                    RichText::new("File contains no supported audio tracks")
                        .heading()
                        .color(Color32::RED),
                );
            }
            CreateDecoderError::UnknownDuration => {
                ui.label(
                    RichText::new("Unable to know duration of file")
                        .heading()
                        .color(Color32::RED),
                );
            }
            CreateDecoderError::UnknownCodec => {
                ui.label(
                    RichText::new("Unknown audio codec")
                        .heading()
                        .color(Color32::RED),
                );
            }
        }
    }
}

pub struct AudioDecoder {
    decoder: Box<dyn Decoder>,
    format: Box<dyn FormatReader>,
    track_id: u32,
    track_frames: u64,
}

impl AudioDecoder {
    // TODO: make last lint global?
    pub fn create_for_file(path: PathBuf) -> Result<(AudioDecoder, PathBuf), CreateDecoderError> {
        // Verify file
        // path.extension()
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(io_error) => return Err(CreateDecoderError::OpenFile(path, io_error)),
        };

        let stream = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|os| os.to_str()) {
            hint.with_extension(extension);
        };

        let mut probe = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|_| CreateDecoderError::UnsupportedAudioFormat)?;

        dbg!(probe.metadata.get());
        let format = probe.format;

        // TODO: track selection
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or(CreateDecoderError::NoSupportedAudioTrack)?;

        let track_frames = track
            .codec_params
            .n_frames
            .ok_or(CreateDecoderError::UnknownDuration)?;

        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|_| CreateDecoderError::UnknownCodec)?;

        Ok((
            AudioDecoder {
                track_id: track.id,
                track_frames,
                decoder,
                format,
            },
            path,
        ))
    }
}

impl AudioDecoder {
    // TODO: channel select/multi channel
    pub fn decode(mut self, progress_callback: &dyn Fn(f32)) -> Waveform<'static> {
        let mut spec = None;
        let mut sample_buf = None;
        let mut samples = Vec::new();

        // The decode loop.
        loop {
            // Get the next packet from the media format.
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                // Err(symphonia::core::errors::Error::ResetRequired) => {
                //     // The track list has been changed. Re-examine it and create a new set of decoders,
                //     // then restart the decode loop. This is an advanced feature and it is not
                //     // unreasonable to consider this "the end." As of v0.5.0, the only usage of this is
                //     // for chained OGG physical streams.
                //     unimplemented!();
                // }
                Err(symphonia::core::errors::Error::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    info!("Reached end of file");
                    break;
                }
                Err(err) => {
                    // A unrecoverable error occured, halt decoding.
                    panic!("{}", err);
                }
            };

            progress_callback(packet.ts() as f32 / self.track_frames as f32);

            // Consume any new metadata that has been read since the last packet.
            while !self.format.metadata().is_latest() {
                // Pop the old head of the metadata queue.
                self.format.metadata().pop();

                // Consume the new metadata at the head of the metadata queue.
                // TODO: process metadata
            }

            // If the packet does not belong to the selected track, skip over it.
            if packet.track_id() != self.track_id {
                continue;
            }

            // Decode the packet into audio samples.
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = spec.get_or_insert(*decoded.spec());

                    let sample_buf = sample_buf.get_or_insert_with(|| {
                        SampleBuffer::<f32>::new(decoded.capacity() as u64, *spec)
                    });

                    sample_buf.copy_planar_ref(decoded);

                    samples.extend_from_slice(
                        &sample_buf.samples()[..sample_buf.len() / spec.channels.count()],
                    );
                }
                // Err(symphonia::core::errors::Error::IoError(_)) => {
                //     // The packet failed to decode due to an IO error, skip the packet.
                //     continue;
                // }
                // Err(symphonia::core::errors::Error::DecodeError(_)) => {
                //     // The packet failed to decode due to invalid data, skip the packet.
                //     continue;
                // }
                Err(err) => {
                    // An unrecoverable error occurred, halt decoding.
                    panic!("{}", err);
                }
            }
        }

        let spec = spec.expect("encountered no packets");

        let waveform = Waveform::new(samples, spec.rate);

        // Sanity check
        debug_assert_eq!(waveform.len() as u64, self.track_frames);

        waveform
    }
}

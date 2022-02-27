use std::{
    fmt::{self, Debug},
    iter,
    sync::mpsc::{self, Sender, TryRecvError},
};

use color_eyre::eyre::{Context, ContextCompat};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    PlayStreamError, Stream, StreamConfig,
};
use tracing::{error, trace};

use crate::waveform::Waveform;

pub struct AudioSink {
    output_stream: Stream,
    config: StreamConfig,
    samples_sender: Sender<Waveform<'static>>,
}

impl Debug for AudioSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioSink").finish()
    }
}

impl AudioSink {
    pub fn new() -> color_eyre::Result<Self> {
        let host = cpal::default_host();

        let output_device = host
            .default_output_device()
            .wrap_err("no default output device")?;

        let config: StreamConfig = output_device
            .default_output_config()
            .wrap_err("no default output config")?
            .into();

        let (samples_sender, samples_receiver) = mpsc::channel::<Waveform<'static>>();

        let output_stream = output_device
            .build_output_stream(
                &config,
                {
                    // Mutable closure state
                    let mut working_samples = Vec::new();

                    // Stream configuration
                    let config = dbg!(config.clone());
                    move |data: &mut [f32], _info| {
                        if working_samples.is_empty() {
                            let new_samples =
                                samples_receiver.recv().expect("samples channel closed");

                            assert_eq!(new_samples.sample_rate(), config.sample_rate.0);

                            trace!("Received {} new samples", new_samples.len());

                            working_samples = new_samples.as_samples();
                        }

                        // Happy path if one channel
                        if config.channels == 1 {
                            let length = data.len().min(working_samples.len());

                            data.copy_from_slice(&working_samples[..length]);

                            // Remove the copied samples
                            working_samples.drain(..length);

                            return;
                        }

                        // Normal path for multi-channel
                        let windows = data.chunks_exact_mut(config.channels.into());
                        let length = windows.len().min(working_samples.len());
                        let drain = working_samples.drain(..length);

                        for (frame, value) in windows.zip(drain.chain(iter::repeat(0.0))) {
                            for sample in frame {
                                *sample = value;
                            }
                        }
                    }
                },
                |err| {
                    error!(%err, "an error occurred on the output stream");
                },
            )
            .wrap_err("failed to build output stream")?;

        Ok(Self {
            output_stream,
            samples_sender,
            config,
        })
    }

    pub fn queue(&self, waveform: &Waveform<'_>) -> Result<bool, PlayStreamError> {
        let resampled_waveform = waveform.resample(self.config.sample_rate.0);

        let send_result = self.samples_sender.send(resampled_waveform);

        self.output_stream.play()?;

        Ok(send_result.is_ok())
    }
}

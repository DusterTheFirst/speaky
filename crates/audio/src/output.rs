use std::{
    fmt::{self, Debug},
    iter,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Sender, TryRecvError},
        Arc, Once,
    },
};

use color_eyre::eyre::{Context, ContextCompat};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Stream, StreamConfig,
};
use tracing::{debug, error, trace};

use crate::waveform::Waveform;

#[derive(Debug, Clone, Copy)]
pub enum AudioSinkProgress {
    Samples(usize),
    Finished,
}

type AudioSinkCallback = Box<dyn Fn(AudioSinkProgress) + Send>;

pub struct AudioSink {
    // FIXME: channels are broken on web assembly due to lack of condvar support.
    // TODO: use a mutex instead
    samples_sender: Sender<(Waveform<'static>, AudioSinkCallback)>,
    config: StreamConfig,

    queue_length: Arc<AtomicUsize>,

    // Field (drop) ordering here is very important, the sender must be dropped
    // before the stream can be dropped to prevent deadlocking
    _output_stream: Stream,
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

        let (samples_sender, samples_receiver) =
            mpsc::channel::<(Waveform<'static>, AudioSinkCallback)>();

        let queue_length = Arc::new(AtomicUsize::new(0));

        let output_stream = output_device
            .build_output_stream(
                &config,
                {
                    // Mutable closure state
                    let mut starting_samples = 0;
                    let mut working_samples = Vec::new();
                    let mut working_callback: AudioSinkCallback = Box::new(|_| {}); // TODO: Option?

                    // Immutable closure state
                    let config = config.clone();
                    let queue_length = queue_length.clone();

                    let mut playing = false;

                    // TODO: clean up this closure
                    move |data: &mut [f32], _info| {
                        if working_samples.is_empty() {
                            if playing {
                                queue_length.fetch_update(
                                    Ordering::SeqCst,
                                    Ordering::SeqCst,
                                    |queue_length| Some(queue_length.saturating_sub(1))
                                ).ok();
                                working_callback(AudioSinkProgress::Finished);
                            }

                            playing = false;

                            match samples_receiver.try_recv() {
                                Ok((new_samples, new_callback)) => {
                                    assert_eq!(new_samples.sample_rate(), config.sample_rate.0);

                                    trace!("Received {} new samples", new_samples.len());

                                    working_samples = new_samples.as_samples();
                                    working_callback = new_callback;
                                    starting_samples = working_samples.len();
                                },
                                Err(e) => {
                                    data.fill(0.0);

                                    match e {
                                        TryRecvError::Empty => std::hint::spin_loop(),
                                        TryRecvError::Disconnected => {
                                            static ONCE: Once = Once::new();

                                            ONCE.call_once(|| {
                                                debug!("Sample channel has hung up, looping until the stream closes");
                                            });
                                        },
                                    }

                                    return;
                                },
                            }
                        }
                        playing = true;

                        // Run the callback
                        // TODO: Deal with resampling
                        working_callback(AudioSinkProgress::Samples(starting_samples - working_samples.len()));

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

        output_stream
            .play()
            .wrap_err("failed to start the output stream")?;

        Ok(Self {
            queue_length,
            _output_stream: output_stream,
            samples_sender,
            config,
        })
    }

    pub fn queue_length(&self) -> usize {
        self.queue_length.load(Ordering::SeqCst)
    }

    pub fn playing(&self) -> bool {
        self.queue_length() >= 1
    }

    pub fn queue(
        &self,
        waveform: &Waveform<'_>,
        callback: impl Fn(AudioSinkProgress) + Send + 'static,
    ) -> bool {
        let resampled_waveform = waveform.resample(self.config.sample_rate.0);

        let send_result = self
            .samples_sender
            .send((resampled_waveform, Box::new(callback)));

        self.queue_length.fetch_add(1, Ordering::SeqCst);

        send_result.is_ok()
    }
}

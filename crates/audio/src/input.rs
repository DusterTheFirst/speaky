use color_eyre::eyre::{Context, ContextCompat};
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleRate, StreamConfig, StreamError,
};
use tracing::error;

pub fn read_one_second() -> color_eyre::Result<(Vec<f32>, SampleRate)> {
    let host = cpal::default_host();

    let input_device = host
        .default_input_device()
        .wrap_err("failed to get the default input device")?;

    let config: StreamConfig = input_device
        .default_input_config()
        .wrap_err("failed to get default input config")?
        .into();

    let (send, recv) = std::sync::mpsc::channel();

    let input_stream = input_device
        .build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                send.send(data.to_vec()).ok();
            },
            |err: StreamError| {
                error!(%err, "an error occurred on the input stream");
            },
        )
        .wrap_err("failed to build input stream")?;

    input_stream.play()?;

    std::thread::sleep(std::time::Duration::from_secs(1));

    drop(input_stream);

    Ok((
        recv.iter()
            .flatten()
            .step_by(config.channels as usize)
            .map(|x| x * 10.0)
            .collect::<Vec<_>>(),
        config.sample_rate,
    ))
}

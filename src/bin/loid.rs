use std::{sync::Arc, thread};

use eframe::{
    egui::{
        plot::{Bar, BarChart, Legend, Plot, Points, Value, Values},
        Button, CentralPanel, CtxRef,
    },
    epi::{App, Frame},
    NativeOptions,
};
use microfft::real::rfft_4096;
use num_complex::Complex;
use rodio::{buffer::SamplesBuffer, source::Buffered, OutputStream, Sink, Source};
use speaky::{load_language, setup_tts, synthesize};

fn main() {
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();

    let sink = Sink::try_new(&stream_handle).unwrap();

    let resources = load_language("en-US").unwrap();

    let mut engine = setup_tts(resources);

    let speech = synthesize(&mut engine, "Some").buffered();

    // let sample_rate = speech.sample_rate();
    let samples: Vec<f32> = speech.clone().convert_samples().collect();

    let mut samples: [f32; 4096] = samples[..4096].try_into().unwrap();

    let spectrum = rfft_4096(&mut samples);

    // TODO: more idiomatic way?
    let spectrum = unsafe { std::mem::transmute(samples) };

    // dbg!(amplitudes.iter().max());

    // let pow_2_len = 1 << (usize::BITS - samples.len().leading_zeros() - 1);

    // let hann_window = hann_window(&samples[..pow_2_len.min(4096)]);

    // let spectrum_hann_window = samples_fft_to_spectrum(
    //     &hann_window,
    //     sample_rate,
    //     FrequencyLimit::All,
    //     None,
    // )
    // .unwrap();

    // dbg!(spectrum_hann_window.max());

    // for (fr, fr_val) in spectrum_hann_window.data().iter() {
    //     println!("{}Hz => {}", fr, fr_val)
    // }

    // sink.append(speech);

    // sink.sleep_until_end();

    eframe::run_native(
        Box::new(Loid {
            audio_sink: Arc::new(sink),

            speech,

            spectrum,
        }),
        NativeOptions::default(),
    )
}

struct Loid {
    audio_sink: Arc<Sink>,

    speech: Buffered<SamplesBuffer<i16>>,

    spectrum: [Complex<f32>; 2048],
}

impl App for Loid {
    fn update(&mut self, ctx: &CtxRef, frame: &Frame) {
        CentralPanel::default().show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(self.audio_sink.empty(), Button::new("Play"))
                    .clicked()
                {
                    self.audio_sink.append(self.speech.clone());

                    thread::spawn({
                        let audio_sink = self.audio_sink.clone();
                        let frame = frame.clone();
                        move || {
                            audio_sink.sleep_until_end();
                            frame.request_repaint();
                        }
                    });
                }
            });

            // FIXME: heavy on the iterators I think
            Plot::new("samples")
                .height(ui.available_height() / 2.0)
                .center_y_axis(true)
                .show(ui, |ui| {
                    let sampling_rate = self.speech.sample_rate();

                    ui.points(
                        Points::new(Values::from_values_iter({
                            self.speech
                                .clone()
                                .convert_samples::<f32>()
                                .enumerate()
                                .map(move |(n, x)| Value::new(n as f32 / sampling_rate as f32, x))
                        }))
                        .stems(0.0),
                    )
                });

            Plot::new("frequencies")
                .height(ui.available_height())
                .legend(Legend::default())
                .show(ui, |ui| {
                    let amplitudes = self.spectrum.iter().map(|complex| complex.norm());

                    ui.bar_chart(
                        BarChart::new(
                            amplitudes
                                .enumerate()
                                .map(|(n, amp)| Bar::new(n as f64, amp as f64))
                                .collect(),
                        )
                        .name("Amplitude"),
                    );

                    let phases = self.spectrum.iter().map(|complex| complex.arg());

                    ui.bar_chart(
                        BarChart::new(
                            phases
                                .enumerate()
                                .map(|(n, amp)| Bar::new(n as f64, amp as f64))
                                .collect(),
                        )
                        .name("Phase"),
                    );
                });
        });
    }

    fn name(&self) -> &str {
        "Shitty Loid"
    }
}

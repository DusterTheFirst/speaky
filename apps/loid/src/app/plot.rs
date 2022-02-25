use audio::waveform::Waveform;
use eframe::{
    egui::{
        plot::{Bar, BarChart, Legend, Line, Plot, PlotUi, Points, Text, VLine, Value, Values},
        RichText, Ui,
    },
    emath::Align2,
    epaint::Color32,
};
use spectrum::{Spectrum, Window};

pub fn waveform_display(
    ui: &mut Ui,
    waveform: &Waveform,
    cursor: usize,
    playback_head: usize,
    window_width: usize,
    hop_frac: usize,
    (line, stems): (bool, bool),
) {
    Plot::new("samples")
        .center_y_axis(true)
        .legend(Legend::default())
        .include_y(1.0)
        .include_y(-1.0)
        .show(ui, |ui| {
            point_line(
                ui,
                "Original waveform",
                Values::from_values_iter(waveform.time_domain().map(|(x, y)| Value::new(x, y))),
                (line, stems),
            );

            // TODO:
            // ui.points(
            //     Points::new(Values::from_values_iter(
            //         reconstructed.time_domain().map(|(x, y)| Value::new(x, y)),
            //     ))
            //     .name("Reconstructed Samples")
            //     .stems(0.0),
            // );

            ui.vline(
                VLine::new(waveform.time_from_sample(cursor))
                    .color(Color32::DARK_GREEN)
                    .width(2.5)
                    .name("Start of window"),
            );
            ui.vline(
                VLine::new(waveform.time_from_sample(cursor + window_width))
                    .color(Color32::DARK_RED)
                    .width(1.5)
                    .name("End of window"),
            );
            ui.vline(
                VLine::new(waveform.time_from_sample(cursor + window_width / hop_frac))
                    .color(Color32::GOLD)
                    .name("Start of next window"),
            );
            ui.vline(
                VLine::new(waveform.time_from_sample(playback_head))
                    .color(Color32::LIGHT_BLUE)
                    .name("Playback head"),
            );
        });
}

pub fn window_display(
    ui: &mut Ui,
    waveform: &Waveform,
    (window, window_width): (Window, usize),
    reconstructed: &Waveform,
    hop_frac: usize,
    (line, stems): (bool, bool),
) {
    Plot::new("window_samples")
        .center_y_axis(true)
        .legend(Legend::default())
        .include_y(1.0)
        .include_y(-1.0)
        .show(ui, |ui| {
            point_line(
                ui,
                "Original samples",
                Values::from_ys_f32(waveform.samples()),
                (line, stems),
            );

            ui.line(
                Line::new(Values::from_values_iter(
                    window
                        .into_iter(window_width)
                        .enumerate()
                        .map(|(i, w)| Value::new(i as f32, w)),
                ))
                .name("Window"),
            );

            point_line(
                ui,
                "Windowed samples",
                Values::from_values_iter(
                    waveform
                        .samples()
                        .iter()
                        .zip(window.into_iter(window_width))
                        .enumerate()
                        .map(|(i, (sample, w))| Value::new(i as f32, w * sample)),
                ),
                (line, stems),
            );

            point_line(
                ui,
                "Shifted samples",
                Values::from_ys_f32(reconstructed.samples()),
                (line, stems),
            );

            ui.vline(VLine::new((window_width / hop_frac) as f32).name("Start of next window"));
        });
}

fn point_line(ui: &mut PlotUi, name: &str, series: Values, (line, stems): (bool, bool)) {
    if line {
        let line = Line::new(series).name(name);

        ui.line(if stems { line.fill(0.0) } else { line });
    } else {
        let points = Points::new(series).name(name);

        ui.points(if stems { points.stems(0.0) } else { points });
    }
}

pub fn spectrum_display(
    ui: &mut Ui,
    spectrum: &Spectrum,
    shifted_spectrum: &Spectrum,
    full_spectrum: bool,
    phase: bool,
    decibels: bool,
) {
    Plot::new("frequencies")
        .legend(Legend::default())
        .center_y_axis(true)
        .show(ui, |ui| {
            display_spectrum(
                ui,
                spectrum,
                "Frequency spectrum",
                full_spectrum,
                phase,
                decibels,
            );

            display_spectrum(
                ui,
                shifted_spectrum,
                "Shifted frequency spectrum",
                full_spectrum,
                phase,
                decibels,
            );
        });
}

fn display_spectrum(
    ui: &mut PlotUi,
    spectrum: &Spectrum,
    title: &str,
    full_spectrum: bool,
    phase: bool,
    decibels: bool,
) {
    // TODO: DECIBELS

    #[inline(always)]
    fn map(
        iterator: impl Iterator<Item = f32>,
        freq: impl Fn(usize) -> f64,
        db: impl Fn(f32) -> f32,
    ) -> Vec<Bar> {
        iterator
            .enumerate()
            .map(|(bucket, mag)| Bar::new(freq(bucket), db(mag) as f64))
            .collect()
    }

    let db = |mag: f32| -> f32 {
        if decibels {
            20.0 * if mag == 0.0 { 0.0 } else { mag.log10() }
        } else {
            mag
        }
    };

    let freq = |b| spectrum.freq_from_bucket(b);

    let buckets = match (phase, full_spectrum) {
        (true, true) => map(&mut spectrum.phases(), freq, db),
        (true, false) => map(spectrum.phases_real(), freq, db),
        (false, true) => map(spectrum.amplitudes(), freq, db),
        (false, false) => map(spectrum.amplitudes_real(), freq, db),
    };

    ui.bar_chart(
        BarChart::new(buckets)
            .width(spectrum.freq_resolution())
            .name(&title),
    );

    if !phase {
        if let Some((bucket, max)) = spectrum.main_frequency() {
            let freq = spectrum.freq_from_bucket(bucket);

            ui.text(
                Text::new(
                    Value::new(freq, db(max)),
                    RichText::new(format!("{:.2}Hz", freq)).monospace(),
                )
                .anchor(Align2::CENTER_BOTTOM),
            )
        }
    }
}

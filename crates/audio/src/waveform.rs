use std::{borrow::Cow, f32::consts, slice::SliceIndex};

use lerp::Lerp;

#[derive(Debug)]
pub struct Waveform<'s> {
    samples: Cow<'s, [f32]>,
    sample_rate: u32,
}

impl Waveform<'static> {
    pub const CD_SAMPLE_RATE: u32 = 44_100;

    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples: Cow::Owned(samples),
            sample_rate,
        }
    }

    pub fn sine_wave(frequency: f32, duration: f32, sample_rate: u32) -> Self {
        let samples_len = (duration * sample_rate as f32).round() as u32;

        let samples = (0..samples_len)
            .map(|n| (frequency * consts::TAU * (n as f32 / sample_rate as f32)).sin())
            .collect();

        Self {
            samples,
            sample_rate,
        }
    }

    pub fn as_samples(self) -> Vec<f32> {
        match self.samples {
            Cow::Borrowed(_) => unreachable!(),
            Cow::Owned(vec) => vec,
        }
    }
}

#[cfg(test)]
mod test {
    use super::Waveform;

    #[test]
    fn as_samples() {
        let waveform = Waveform::sine_wave(100.0, 1.0, Waveform::CD_SAMPLE_RATE);

        assert_eq!(waveform.len(), waveform.as_samples().len());
    }
}

impl Waveform<'_> {
    /// Prefer [`Self::as_samples`] if you have a waveform with a static lifetime
    /// or [`Self::samples`] if you do not need ownership
    pub fn into_samples(self) -> Vec<f32> {
        self.samples.into_owned()
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn samples_iter(&self) -> impl ExactSizeIterator<Item = f32> + '_ {
        self.samples.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn duration(&self) -> f32 {
        self.time_from_sample(self.len())
    }

    pub fn time_from_sample(&self, sample: usize) -> f32 {
        sample as f32 / self.sample_rate as f32
    }

    pub fn time_domain(&self) -> impl ExactSizeIterator<Item = (f32, f32)> + '_ {
        self.samples_iter()
            .enumerate()
            .map(|(sample, x)| (self.time_from_sample(sample), x))
    }

    pub fn to_owned(&self) -> Waveform<'static> {
        Waveform {
            sample_rate: self.sample_rate,
            samples: Cow::Owned(self.samples.clone().into_owned()),
        }
    }

    #[must_use = "Waveform::slice() creates a new waveform over the shortened range"]
    pub fn slice(&self, range: impl SliceIndex<[f32], Output = [f32]>) -> Waveform {
        Waveform {
            sample_rate: self.sample_rate,
            samples: Cow::Borrowed(&self.samples[range]),
        }
    }

    #[must_use = "Waveform::resample() does not modify the provided waveform"]
    pub fn resample(&self, new_sample_rate: u32) -> Waveform<'static> {
        let new_sample_len =
            (self.time_from_sample(self.len() - 1) * new_sample_rate as f32) as usize;

        let mut resampled = vec![0.0; new_sample_len];

        // Resample the waveform
        for (n, sample) in resampled.iter_mut().enumerate() {
            // Calculate where this sample lies
            let virtual_sample = (n as f32 / new_sample_rate as f32) * self.sample_rate as f32;

            // Get the sample before and after this fractional sample
            let before_sample = virtual_sample.floor() as usize;
            let after_sample = virtual_sample.ceil() as usize;

            // Get the percentage between the two samples this sample is
            let lerp_frac = virtual_sample.fract();

            // Linearly interpolate between the two
            *sample = Lerp::lerp(
                self.samples[before_sample],
                self.samples[after_sample],
                lerp_frac,
            );
        }

        Waveform {
            sample_rate: new_sample_rate,
            samples: Cow::Owned(resampled),
        }
    }
}

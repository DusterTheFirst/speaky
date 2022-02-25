use std::{borrow::Cow, slice::SliceIndex};

#[derive(Debug)]
pub struct Waveform<'s> {
    samples: Cow<'s, [f32]>,
    sample_rate: u32,
}

impl Waveform<'_> {
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Waveform<'static> {
        Waveform {
            samples: Cow::Owned(samples),
            sample_rate,
        }
    }

    pub fn slice(&self, range: impl SliceIndex<[f32], Output = [f32]>) -> Waveform {
        Waveform {
            sample_rate: self.sample_rate,
            samples: Cow::Borrowed(&self.samples[range]),
        }
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
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

    pub fn time_from_sample(&self, sample: usize) -> f32 {
        sample as f32 / self.sample_rate as f32
    }

    pub fn time_domain(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.samples
            .iter()
            .enumerate()
            .map(|(sample, x)| (self.time_from_sample(sample), *x))
    }
}
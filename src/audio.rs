use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, StreamConfig};

pub const SAMPLE_RATE: u32 = 16_000;

/// Live input level (RMS of the latest buffer), stored as `f32` bits for the overlay.
pub type Level = Arc<AtomicU32>;

/// The microphone stays open only while a recording runs.
pub struct Recorder {
    _stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
}

impl Recorder {
    pub fn start(level: Level, max_secs: u64) -> Result<Self> {
        let device = cpal::default_host()
            .default_input_device()
            .context("no microphone found")?;
        let wanted = StreamConfig {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            buffer_size: BufferSize::Default,
        };
        match Self::open(&device, wanted, Arc::clone(&level), max_secs) {
            Ok(recorder) => Ok(recorder),
            Err(_) => {
                let config = device
                    .default_input_config()
                    .context("microphone has no usable format")?
                    .config();
                Self::open(&device, config, level, max_secs)
            }
        }
    }

    fn open(
        device: &cpal::Device,
        config: StreamConfig,
        level: Level,
        max_secs: u64,
    ) -> Result<Self> {
        let samples = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&samples);
        let channels = usize::from(config.channels.max(1));
        let limit = usize::try_from(u64::from(config.sample_rate) * max_secs).unwrap_or(usize::MAX);
        let stream = device.build_input_stream::<f32, _, _>(
            config,
            move |data, _| {
                let mono: Vec<f32> = data
                    .chunks(channels)
                    .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
                    .collect();
                level.store(rms(&mono).to_bits(), Ordering::Relaxed);
                let mut samples = sink.lock().unwrap();
                let room = limit.saturating_sub(samples.len());
                samples.extend_from_slice(&mono[..mono.len().min(room)]);
            },
            |err| eprintln!("pickscribe: microphone error: {err}"),
            None,
        )?;
        stream.play()?;
        Ok(Self {
            _stream: stream,
            samples,
            rate: config.sample_rate,
        })
    }

    /// Closes the microphone and returns the recording as 16 kHz mono.
    pub fn finish(self) -> Vec<f32> {
        let rate = self.rate;
        drop(self._stream);
        let samples = std::mem::take(&mut *self.samples.lock().unwrap());
        resample(&samples, rate, SAMPLE_RATE)
    }
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Loudest 100 ms window, used to skip recordings that hold no speech.
pub fn peak_window_rms(samples: &[f32]) -> f32 {
    samples
        .chunks(SAMPLE_RATE as usize / 10)
        .map(rms)
        .fold(0.0, f32::max)
}

fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let step = f64::from(from) / f64::from(to);
    let len = (samples.len() as f64 / step) as usize;
    (0..len)
        .map(|i| {
            let pos = i as f64 * step;
            let index = pos as usize;
            let next = samples.get(index + 1).copied().unwrap_or(samples[index]);
            let frac = (pos - index as f64) as f32;
            samples[index] + (next - samples[index]) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_keeps_duration_and_shape() {
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48_000.0).sin()).collect();
        let output = resample(&input, 48_000, SAMPLE_RATE);
        assert_eq!(output.len(), 16_000);
        assert!((output[8_000] - input[24_000]).abs() < 1e-4);
        assert_eq!(
            resample(&input[..10], SAMPLE_RATE, SAMPLE_RATE),
            &input[..10]
        );
    }

    #[test]
    fn silence_has_no_peak_and_speech_does() {
        assert_eq!(peak_window_rms(&[0.0; 32_000]), 0.0);
        let mut audio = vec![0.0; 32_000];
        audio[16_000..17_600].fill(0.5);
        assert!((peak_window_rms(&audio) - 0.5).abs() < 1e-6);
    }
}

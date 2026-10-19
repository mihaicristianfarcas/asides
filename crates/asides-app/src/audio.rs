//! Microphone capture: mono f32 samples at the device's native rate, kept in memory.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

pub struct Recorder {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    pub sample_rate: u32,
}

impl Recorder {
    /// Starts capturing from `device_name`, or the system default input when empty or missing.
    pub fn start(device_name: &str) -> Result<Recorder> {
        let host = cpal::default_host();
        let named = (!device_name.is_empty())
            .then(|| host.input_devices().ok()?.find(|d| name_of(d).as_deref() == Some(device_name)))
            .flatten();
        let device = named.or_else(|| host.default_input_device()).context("no microphone found")?;
        let supported = device.default_input_config().context("the microphone has no usable format")?;
        let channels = supported.channels() as usize;
        let sample_rate = supported.sample_rate();
        let samples = Arc::new(Mutex::new(Vec::with_capacity(sample_rate as usize * 120)));
        let config = supported.config();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, channels, samples.clone()),
            SampleFormat::I16 => build::<i16>(&device, config, channels, samples.clone()),
            SampleFormat::I32 => build::<i32>(&device, config, channels, samples.clone()),
            SampleFormat::U16 => build::<u16>(&device, config, channels, samples.clone()),
            other => bail!("unsupported microphone sample format {other:?}"),
        }?;
        stream.play().context("could not start the microphone")?;
        Ok(Recorder { stream, samples, sample_rate })
    }

    /// How loud the last 50 ms were, from 0 (−50 dB or quieter) to 1 (−10 dB or louder): the pill's
    /// microphone fills with it.
    pub fn level(&self) -> f32 {
        let Ok(samples) = self.samples.lock() else { return 0.0 };
        let recent = &samples[samples.len().saturating_sub(self.sample_rate as usize / 20)..];
        if recent.is_empty() {
            return 0.0;
        }
        let rms = (recent.iter().map(|s| s * s).sum::<f32>() / recent.len() as f32).sqrt();
        ((20.0 * rms.max(1e-6).log10() + 50.0) / 40.0).clamp(0.0, 1.0)
    }

    /// Stops capturing and returns the mono samples and their rate.
    pub fn finish(self) -> (Vec<f32>, u32) {
        drop(self.stream);
        let samples = std::mem::take(&mut *self.samples.lock().unwrap_or_else(|e| e.into_inner()));
        (samples, self.sample_rate)
    }
}

pub fn input_device_names() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devices| devices.filter_map(|d| name_of(&d)).collect())
        .unwrap_or_default()
}

fn name_of(device: &cpal::Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_string())
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let Ok(mut out) = samples.lock() else { return };
            for frame in data.chunks(channels) {
                let sum: f32 = frame.iter().map(|s| f32::from_sample(*s)).sum();
                out.push(sum / channels as f32);
            }
        },
        |err| eprintln!("microphone stream error: {err}"),
        None,
    )?;
    Ok(stream)
}

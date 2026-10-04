//! CLI-only: play samples through the default audio output (`put -f wav`).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

/// The default output device: its name and native sample rate.
pub struct Output {
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
}

impl Output {
    pub fn open_default() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| {
            anyhow!("no audio output device found (write a WAV file with `sde convert <file> -f wav` instead)")
        })?;
        let config = device.default_output_config().context("cannot query the audio output")?;
        Ok(Output { device, config })
    }

    pub fn name(&self) -> String {
        self.device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "default output".into())
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate()
    }

    /// Play mono `samples` (at [`Self::sample_rate`]) to the end, calling `progress`
    /// with the fraction played about ten times a second.
    pub fn play(&self, samples: Vec<f32>, progress: impl FnMut(f64)) -> Result<()> {
        let config: StreamConfig = self.config.into();
        match self.config.sample_format() {
            SampleFormat::I8 => self.run::<i8>(config, samples, progress),
            SampleFormat::I16 => self.run::<i16>(config, samples, progress),
            SampleFormat::I32 => self.run::<i32>(config, samples, progress),
            SampleFormat::U8 => self.run::<u8>(config, samples, progress),
            SampleFormat::U16 => self.run::<u16>(config, samples, progress),
            SampleFormat::U32 => self.run::<u32>(config, samples, progress),
            SampleFormat::F32 => self.run::<f32>(config, samples, progress),
            SampleFormat::F64 => self.run::<f64>(config, samples, progress),
            f => bail!("unsupported audio output sample format {f}"),
        }
    }

    fn run<T>(&self, config: StreamConfig, samples: Vec<f32>, mut progress: impl FnMut(f64)) -> Result<()>
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = config.channels as usize;
        let total = samples.len();
        let pos = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let (p, f) = (pos.clone(), failed.clone());
        let stream = self.device.build_output_stream(
            config,
            move |out: &mut [T], _| {
                let mut i = p.load(Ordering::Relaxed);
                for frame in out.chunks_mut(channels) {
                    let v = T::from_sample(samples.get(i).copied().unwrap_or(0.0));
                    frame.iter_mut().for_each(|s| *s = v);
                    i += 1;
                }
                p.store(i, Ordering::Relaxed);
            },
            move |e| {
                eprintln!("audio output error: {e}");
                f.store(true, Ordering::Relaxed);
            },
            None,
        )?;
        stream.play()?;
        // Run on past the end for a moment so the device's buffer drains.
        let tail = (self.sample_rate() as usize) / 2;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            let i = pos.load(Ordering::Relaxed);
            progress((i as f64 / total.max(1) as f64).min(1.0));
            if failed.load(Ordering::Relaxed) {
                bail!("audio output failed during playback");
            }
            if i >= total + tail {
                return Ok(());
            }
        }
    }
}

/// Linear-interpolation resampling (for playing a WAV at the device's rate).
pub fn resample(samples: Vec<f32>, from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples;
    }
    let step = from as f64 / to as f64;
    let n = (samples.len() as f64 / step) as usize;
    (0..n)
        .map(|i| {
            let x = i as f64 * step;
            let k = x as usize;
            let a = samples[k.min(samples.len() - 1)];
            let b = samples[(k + 1).min(samples.len() - 1)];
            a + (b - a) * (x - k as f64) as f32
        })
        .collect()
}

//! Sound, and the clock the picture follows.
//!
//! The sound card asks for samples ([`Mixer::fill`]); each ask moves the timeline on by the
//! samples it took, so the position is what has been played: the master clock. Each audio clip
//! is an ffmpeg decoding PCM (stereo f32 at the card's rate) into a [`Source`] that knows the
//! timeline sample its next sample belongs at; the mixer adds up the sources due at each sample,
//! drops what arrived too late, and plays silence for what hasn't arrived, so the clips stay in
//! sync with the clock whatever the decoders do.

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use super::ffmpeg::{self, Process};
use crate::state::ServerHttp;

/// Stereo frames per chunk a decoder hands over.
const CHUNK: usize = 1024;
/// Chunks decoded ahead (about a second at 48 kHz). A full pipe holds ffmpeg back.
const BUFFERED: usize = 48;

/// What every source plays into: the position on the timeline, in samples at `rate`.
pub struct Mixer {
    pub rate: u32,
    position: AtomicU64,
    playing: AtomicBool,
    /// f32 bits; 0 while muted.
    gain: AtomicU32,
    /// The output stopped (unplugged): the player falls back to the wall clock.
    broken: AtomicBool,
    sources: Mutex<Vec<Source>>,
}

impl Mixer {
    pub fn new(rate: u32) -> Mixer {
        Mixer {
            rate,
            position: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            gain: AtomicU32::new(1f32.to_bits()),
            broken: AtomicBool::new(false),
            sources: Mutex::new(Vec::new()),
        }
    }

    pub fn time(&self) -> f64 {
        self.position.load(Ordering::Acquire) as f64 / self.rate as f64
    }

    /// Moves the clock; the sources, which belong at the old place, go.
    pub fn seek(&self, time: f64) {
        let mut sources = self.sources.lock().unwrap();
        sources.clear();
        self.position.store(self.sample(time), Ordering::Release);
    }

    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::Release);
    }

    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Release);
    }

    pub fn broken(&self) -> bool {
        self.broken.load(Ordering::Acquire)
    }

    pub fn sample(&self, time: f64) -> u64 {
        (time.max(0.) * self.rate as f64).round() as u64
    }

    pub fn has(&self, key: &str) -> bool {
        self.sources
            .lock()
            .unwrap()
            .iter()
            .any(|source| source.key == key)
    }

    pub fn add(&self, source: Source) {
        self.sources.lock().unwrap().push(source);
    }

    /// Keeps the sources `keep` says to, stopping the others' decoders.
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        self.sources
            .lock()
            .unwrap()
            .retain(|source| keep(&source.key));
    }

    /// Whether every source has its first samples (or will never have any).
    pub fn ready(&self) -> bool {
        self.sources.lock().unwrap().iter_mut().all(Source::ready)
    }

    /// The sound card's ask: `out` holds frames of `channels` samples. Silence while paused,
    /// and then the clock stands still.
    pub fn fill(&self, out: &mut [f32], channels: usize) {
        out.fill(0.);
        if !self.playing.load(Ordering::Acquire) || channels == 0 {
            return;
        }
        let frames = out.len() / channels;
        let start = self.position.load(Ordering::Acquire);
        let gain = f32::from_bits(self.gain.load(Ordering::Acquire));
        for source in self.sources.lock().unwrap().iter_mut() {
            source.mix(out, channels, start, gain);
        }
        self.position
            .store(start + frames as u64, Ordering::Release);
    }
}

/// The part of a clip to decode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// Seconds into the source.
    pub seek: f64,
    /// Timeline seconds.
    pub length: f64,
    /// The clip's speed.
    pub speed: f64,
    /// The timeline sample the first sample belongs at.
    pub first: u64,
}

/// One clip's samples, stereo, each knowing where on the timeline it belongs.
pub struct Source {
    pub key: String,
    /// The timeline sample the next sample belongs at.
    next: u64,
    chunks: Receiver<Vec<f32>>,
    chunk: Vec<f32>,
    read: usize,
    ended: bool,
    _process: Option<Process>,
}

impl Source {
    /// Samples from `chunks`, the first belonging at timeline sample `first`.
    pub fn new(key: String, first: u64, chunks: Receiver<Vec<f32>>) -> Source {
        Source {
            key,
            next: first,
            chunks,
            chunk: Vec::new(),
            read: 0,
            ended: false,
            _process: None,
        }
    }

    /// ffmpeg decoding `span` of `url` at `rate` samples a second.
    pub fn decode(
        server: &ServerHttp,
        key: String,
        url: &str,
        span: Span,
        rate: u32,
    ) -> std::io::Result<Source> {
        let Span {
            seek,
            length,
            speed,
            first,
        } = span;
        let mut args = ffmpeg::input_args(server, url, Some(seek));
        args.extend(["-vn".into(), "-t".into(), format!("{:.4}", length.max(0.))]);
        if let Some(tempo) = atempo(speed) {
            args.extend(["-af".into(), tempo]);
        }
        args.extend(["-ac", "2", "-ar"].map(String::from));
        args.extend([
            rate.to_string(),
            "-f".into(),
            "f32le".into(),
            "pipe:1".into(),
        ]);
        let (process, mut stdout) = ffmpeg::spawn(&args)?;
        let (sender, chunks) = sync_channel(BUFFERED);
        std::thread::Builder::new()
            .name("linker-edit-audio".into())
            .spawn(move || {
                let mut bytes = vec![0u8; CHUNK * 2 * 4];
                loop {
                    let mut filled = 0;
                    while filled < bytes.len() {
                        match stdout.read(&mut bytes[filled..]) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => filled += n,
                        }
                    }
                    let samples: Vec<f32> = bytes[..filled - filled % 8]
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|sample| f32::from_le_bytes(*sample))
                        .collect();
                    let last = filled < bytes.len();
                    if (!samples.is_empty() && sender.send(samples).is_err()) || last {
                        return;
                    }
                }
            })?;
        let mut source = Source::new(key, first, chunks);
        source._process = Some(process);
        Ok(source)
    }

    /// The next stereo sample, if decoded.
    fn pop(&mut self) -> Option<(f32, f32)> {
        if self.read >= self.chunk.len() {
            match self.chunks.try_recv() {
                Ok(chunk) => {
                    self.chunk = chunk;
                    self.read = 0;
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    self.ended = true;
                    return None;
                }
            }
        }
        let sample = (self.chunk[self.read], self.chunk[self.read + 1]);
        self.read += 2;
        self.next += 1;
        Some(sample)
    }

    fn ready(&mut self) -> bool {
        if self.read < self.chunk.len() || self.ended {
            return true;
        }
        match self.chunks.try_recv() {
            Ok(chunk) => {
                self.chunk = chunk;
                self.read = 0;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.ended = true;
                true
            }
        }
    }

    /// Adds this source's samples for the frames from timeline sample `start` into `out`.
    fn mix(&mut self, out: &mut [f32], channels: usize, start: u64, gain: f32) {
        for (index, frame) in out.chunks_exact_mut(channels).enumerate() {
            let at = start + index as u64;
            // Too late for these: the decoder fell behind, or started before the clock.
            while self.next < at {
                if self.pop().is_none() {
                    return;
                }
            }
            if self.next > at {
                continue;
            }
            let Some((left, right)) = self.pop() else {
                return;
            };
            if channels == 1 {
                frame[0] += (left + right) * 0.5 * gain;
            } else {
                frame[0] += left * gain;
                frame[1] += right * gain;
            }
        }
    }
}

/// ffmpeg's atempo for a clip's speed, which keeps the pitch as the browser's playbackRate does.
/// One atempo goes from 0.5 to 2, so faster or slower takes several.
pub fn atempo(speed: f64) -> Option<String> {
    let mut speed = speed.abs().clamp(0.01, 100.);
    if (speed - 1.).abs() < 1e-6 {
        return None;
    }
    let mut steps = Vec::new();
    while speed > 2. {
        steps.push("atempo=2".to_string());
        speed /= 2.;
    }
    while speed < 0.5 {
        steps.push("atempo=0.5".to_string());
        speed /= 0.5;
    }
    steps.push(format!("atempo={speed}"));
    Some(steps.join(","))
}

/// The sound card, playing the mixer.
pub struct Output {
    pub mixer: Arc<Mixer>,
    _stream: cpal::Stream,
}

impl Output {
    /// The default output device; an error when there is none, or it won't open.
    pub fn open() -> anyhow::Result<Output> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("no sound output"))?;
        let config = device.default_output_config()?;
        let format = config.sample_format();
        let config: cpal::StreamConfig = config.into();
        let mixer = Arc::new(Mixer::new(config.sample_rate));
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, &mixer)?,
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, &mixer)?,
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, &mixer)?,
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, &mixer)?,
            other => anyhow::bail!("the sound output takes {other} samples"),
        };
        stream.play()?;
        Ok(Output {
            mixer,
            _stream: stream,
        })
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: &Arc<Mixer>,
) -> anyhow::Result<cpal::Stream> {
    let channels = config.channels as usize;
    let (fill, broken) = (mixer.clone(), mixer.clone());
    let mut mixed = Vec::new();
    Ok(device.build_output_stream(
        *config,
        move |out: &mut [T], _| {
            mixed.resize(out.len(), 0f32);
            fill.fill(&mut mixed, channels);
            for (out, sample) in out.iter_mut().zip(&mixed) {
                *out = T::from_sample(sample.clamp(-1., 1.));
            }
        },
        move |error| {
            eprintln!("Sound output: {error}");
            broken.broken.store(true, Ordering::Release);
        },
        None,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source whose samples are their own timeline sample numbers, from `first` on.
    fn counting(first: u64, count: u64) -> Source {
        let (sender, chunks) = sync_channel(64);
        let samples: Vec<f32> = (first..first + count)
            .flat_map(|n| [n as f32, -(n as f32)])
            .collect();
        for chunk in samples.chunks(6) {
            sender.send(chunk.to_vec()).unwrap();
        }
        Source::new(format!("{first}"), first, chunks)
    }

    fn left(out: &[f32]) -> Vec<f32> {
        out.chunks(2).map(|frame| frame[0]).collect()
    }

    #[test]
    fn plays_each_sample_at_its_place_on_the_clock() {
        let mixer = Mixer::new(10);
        mixer.add(counting(3, 4));
        let mut out = vec![0.; 16];

        // Paused: silence, and the clock stands still.
        mixer.fill(&mut out, 2);
        assert_eq!(mixer.time(), 0.);
        assert!(out.iter().all(|sample| *sample == 0.));

        // A clip starting at sample 3 is silent until then and stops when it runs out.
        mixer.set_playing(true);
        mixer.fill(&mut out, 2);
        assert_eq!(left(&out), [0., 0., 0., 3., 4., 5., 6., 0.]);
        assert_eq!(out[7], -3.);
        assert_eq!(mixer.time(), 0.8);
    }

    #[test]
    fn drops_what_came_too_late_and_mixes_at_the_volume() {
        let mixer = Mixer::new(10);
        mixer.seek(0.5);
        // Decoded from sample 2 on, but the clock is at 5 already: 2–4 are dropped.
        mixer.add(counting(2, 10));
        mixer.add(counting(5, 2));
        mixer.set_playing(true);
        mixer.set_gain(0.5);
        let mut out = vec![0.; 4];
        mixer.fill(&mut out, 1);
        // Mono takes the middle of left and right, which are opposite here: silence, in time.
        assert_eq!(out, [0.; 4]);
        let mut out = vec![0.; 8];
        mixer.seek(0.);
        mixer.add(counting(0, 2));
        mixer.add(counting(1, 2));
        mixer.fill(&mut out, 2);
        assert_eq!(left(&out), [0., 0.5 + 0.5, 1., 0.]);
        // A seek clears the sources, which belong where the clock was.
        mixer.seek(0.);
        mixer.fill(&mut out, 2);
        assert!(out.iter().all(|sample| *sample == 0.));
    }

    #[test]
    fn keeps_the_pitch_at_any_speed() {
        assert_eq!(atempo(1.), None);
        assert_eq!(atempo(1.5).as_deref(), Some("atempo=1.5"));
        assert_eq!(atempo(5.).as_deref(), Some("atempo=2,atempo=2,atempo=1.25"));
        assert_eq!(atempo(0.25).as_deref(), Some("atempo=0.5,atempo=0.5"));
    }

    #[test]
    fn decodes_a_clips_sound_into_samples() {
        let Some(file) = super::super::video::tests::red_then_blue() else {
            eprintln!("no ffmpeg: skipped");
            return;
        };
        let server = super::super::video::tests::local();
        let url = file.to_string_lossy();
        let span = Span {
            seek: 0.5,
            length: 1.,
            speed: 1.,
            first: 80,
        };
        let source = Source::decode(&server, "a".into(), &url, span, 8000).unwrap();
        let mut samples = Vec::new();
        while let Ok(chunk) = source.chunks.recv() {
            samples.extend(chunk);
        }
        // A second of stereo at 8 kHz, and the tone (lavfi's sine is at 1/8), not silence.
        assert!(
            (samples.len() as i64 - 16000).abs() < 400,
            "{}",
            samples.len()
        );
        assert!(samples.iter().any(|sample| sample.abs() > 0.05));
    }
}

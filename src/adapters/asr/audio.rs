// Adapted from Yulu pcmResampler.ts/offlineTranscription.ts (MIT, see LICENSE-Yulu).
use crate::core::contracts::*;
use std::{f64::consts::PI, path::Path};

pub const RATE: usize = 16_000;
const TAPS: usize = 127;

/// Source-separated, phase-preserving 127-tap Blackman low-pass conversion.
/// The causal filter retains Yulu's <4ms delay; timestamps are VAD sample times.
struct Resampler {
    rate: usize,
    phase: usize,
    cursor: usize,
    initialized: bool,
    history: Vec<[f64; TAPS]>,
    coefficients: Option<[f64; TAPS]>,
}
impl Resampler {
    fn new(rate: usize, channels: usize) -> Self {
        let coefficients = if rate > RATE {
            let cutoff = 7200.0 / rate as f64;
            let mut c = std::array::from_fn(|i| {
                let distance = i as f64 - (TAPS - 1) as f64 / 2.0;
                let sinc = if distance == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * PI * cutoff * distance).sin() / (PI * distance)
                };
                sinc * (0.42 - 0.5 * (2.0 * PI * i as f64 / (TAPS - 1) as f64).cos()
                    + 0.08 * (4.0 * PI * i as f64 / (TAPS - 1) as f64).cos())
            });
            let sum: f64 = c.iter().sum();
            for x in &mut c {
                *x /= sum;
            }
            Some(c)
        } else {
            None
        };
        Self {
            rate,
            phase: 0,
            cursor: 0,
            initialized: false,
            history: vec![[0.0; TAPS]; channels],
            coefficients,
        }
    }
    fn feed(&mut self, frame: &[i16], out: &mut [Vec<f32>]) {
        for (h, value) in self.history.iter_mut().zip(frame) {
            if !self.initialized {
                h.fill(*value as f64);
            }
            h[self.cursor] = *value as f64;
        }
        self.initialized = true;
        self.phase += RATE;
        while self.phase >= self.rate {
            self.phase -= self.rate;
            for (h, dest) in self.history.iter().zip(out.iter_mut()) {
                let mut value = h[self.cursor];
                if let Some(c) = &self.coefficients {
                    value = c
                        .iter()
                        .enumerate()
                        .map(|(i, c)| c * h[(self.cursor + TAPS - i) % TAPS])
                        .sum();
                } else if self.rate < RATE {
                    let prev = h[(self.cursor + TAPS - 1) % TAPS];
                    value = prev + (value - prev) * (1.0 - self.phase as f64 / RATE as f64);
                }
                // JavaScript Math.round uses floor(x+0.5), including negative halves.
                dest.push((value + 0.5).floor().clamp(-32768.0, 32767.0) as f32 / 32768.0);
            }
        }
        self.cursor = (self.cursor + 1) % TAPS;
    }
}

/// Read a fixed number of input frames per step, retaining filter/phase across steps.
pub struct SourceReader {
    reader: hound::WavReader<std::io::BufReader<std::fs::File>>,
    converter: Resampler,
    remaining: u64,
}
impl SourceReader {
    pub fn open(path: &Path, declared_ms: u64) -> Result<Self> {
        let reader =
            hound::WavReader::open(path).map_err(|_| corrupt("invalid or unreadable WAV"))?;
        let spec = reader.spec();
        if spec.sample_format != hound::SampleFormat::Int
            || spec.bits_per_sample != 16
            || !(1..=2).contains(&spec.channels)
            || !(8000..=192000).contains(&spec.sample_rate)
        {
            return Err(corrupt("ASR requires mono/stereo PCM16 WAV at 8–192kHz"));
        }
        let frames = reader.duration() as u64;
        if (frames * 1000 / spec.sample_rate as u64).abs_diff(declared_ms) > 2
            || reader.len() as u64 != frames * spec.channels as u64
        {
            return Err(corrupt(
                "WAV duration or frame alignment differs from sealed metadata",
            ));
        }
        Ok(Self {
            reader,
            converter: Resampler::new(spec.sample_rate as usize, spec.channels as usize),
            remaining: frames,
        })
    }
    pub fn next_chunk(&mut self) -> Result<Option<Vec<Vec<f32>>>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        let count = self.remaining.min(32768);
        let channels = self.converter.history.len();
        let mut out = vec![Vec::new(); channels];
        let mut samples = self.reader.samples::<i16>();
        for _ in 0..count {
            let mut frame = [0i16; 2];
            for value in &mut frame[..channels] {
                *value = samples
                    .next()
                    .ok_or_else(|| corrupt("truncated WAV frame"))?
                    .map_err(|_| corrupt("invalid WAV samples"))?;
            }
            self.converter.feed(&frame[..channels], &mut out);
        }
        self.remaining -= count;
        Ok(Some(out))
    }
}
/// Private disposable raw float PCM spool. Never writes final artifacts or DB.
pub struct PreparedAudio {
    pub paths: Vec<std::path::PathBuf>,
    pub samples: usize,
    directory: std::path::PathBuf,
}
impl Drop for PreparedAudio {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
pub fn prepare_sources(path: &Path, declared_ms: u64, work: &Path) -> Result<PreparedAudio> {
    use std::{
        fs::{DirBuilder, OpenOptions},
        io::{BufWriter, Write},
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    };
    let mut reader = SourceReader::open(path, declared_ms)?;
    let directory = work.join(format!("asr-{}", uuid::Uuid::new_v4()));
    DirBuilder::new().mode(0o700).create(&directory)?;
    let mut prepared = PreparedAudio {
        paths: vec![],
        samples: 0,
        directory,
    };
    let mut writers = vec![];
    for channel in 0..reader.converter.history.len() {
        let path = prepared.directory.join(if channel == 0 {
            "mic.f32"
        } else {
            "system.f32"
        });
        writers.push(BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?,
        ));
        prepared.paths.push(path);
    }
    while let Some(chunk) = reader.next_chunk()? {
        prepared.samples += chunk[0].len();
        for (source, writer) in chunk.iter().zip(&mut writers) {
            for sample in source {
                writer.write_all(&sample.to_le_bytes())?;
            }
        }
    }
    for writer in &mut writers {
        writer.flush()?;
    }
    Ok(prepared)
}
pub(super) fn corrupt(message: &str) -> OmmError {
    OmmError::new(ErrorCode::Corrupt, false, message)
}

pub fn merge_spans(spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    let mut merged: Vec<(usize, usize)> = vec![];
    for (start, end) in spans {
        if let Some(last) = merged.last_mut() {
            if start.saturating_sub(last.1) < RATE / 5 && end - last.0 <= RATE * 25 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

fn key(s: &str) -> Vec<char> {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || ('\u{3400}'..='\u{9fff}').contains(c))
        .collect()
}
pub fn likely_duplicate(a: &str, b: &str) -> bool {
    let a = key(a);
    let b = key(b);
    if a.len().min(b.len()) < 6 {
        return a == b && a.len() >= 3;
    }
    if a.windows(b.len()).any(|s| s == b) || b.windows(a.len()).any(|s| s == a) {
        return true;
    }
    let aa: std::collections::HashSet<_> = a.windows(2).collect();
    let bb: std::collections::HashSet<_> = b.windows(2).collect();
    2.0 * aa.intersection(&bb).count() as f64 / (aa.len() + bb.len()).max(1) as f64 >= 0.82
}
pub fn deduplicate(mut segments: Vec<TranscriptSegment>) -> Vec<TranscriptSegment> {
    segments.sort_by_key(|s| (s.start_ms, s.end_ms));
    let mut kept: Vec<TranscriptSegment> = vec![];
    for candidate in segments {
        if let Some(i) = kept.iter().position(|s| {
            s.source != candidate.source
                && candidate.start_ms < s.end_ms.saturating_add(1500)
                && candidate.end_ms.saturating_add(1500) > s.start_ms
                && likely_duplicate(&s.text, &candidate.text)
        }) {
            if candidate.source == AudioSource::System {
                kept[i] = candidate;
            }
        } else {
            kept.push(candidate);
        }
    }
    kept.sort_by_key(|s| (s.start_ms, s.end_ms));
    kept
}

#![cfg(feature = "asr")]
use omm::{adapters::asr::audio::*, core::contracts::*};
use std::{f64::consts::PI, path::Path};
fn read_sources(path: &Path, ms: u64) -> Result<Vec<Vec<f32>>> {
    let mut reader = SourceReader::open(path, ms)?;
    let mut out = vec![];
    while let Some(chunk) = reader.next_chunk()? {
        if out.is_empty() {
            out = vec![vec![]; chunk.len()];
        }
        for (source, dest) in chunk.into_iter().zip(&mut out) {
            dest.extend(source);
        }
    }
    Ok(out)
}
fn wav(path: &Path, rate: u32, samples: impl Iterator<Item = (i16, i16)>) {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for (mic, sys) in samples {
        writer.write_sample(mic).unwrap();
        writer.write_sample(sys).unwrap();
    }
    writer.finalize().unwrap();
}
#[test]
fn stereo_pcm16_keeps_source_ownership_and_exact_16k_length() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.wav");
    for rate in [8000, 16000, 44100, 48000] {
        wav(&path, rate, (0..rate).map(|_| (1000, -2000)));
        let sources = read_sources(&path, 1000).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].len(), 16000);
        assert_eq!(sources[1].len(), 16000);
        assert!(sources[0]
            .iter()
            .all(|x| (*x - 1000.0 / 32768.0).abs() < 1e-6));
        assert!(sources[1]
            .iter()
            .all(|x| (*x + 2000.0 / 32768.0).abs() < 1e-6));
    }
    assert!(read_sources(&path, 500).is_err());
}
#[test]
fn antialias_filter_attenuates_out_of_band_system_audio_not_mic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tones.wav");
    wav(
        &path,
        48000,
        (0..48000).map(|i| {
            (
                (12000.0 * (2.0 * PI * 1000.0 * i as f64 / 48000.0).sin()) as i16,
                (12000.0 * (2.0 * PI * 12000.0 * i as f64 / 48000.0).sin()) as i16,
            )
        }),
    );
    let sources = read_sources(&path, 1000).unwrap();
    let rms =
        |s: &[f32]| (s.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
    assert!(rms(&sources[0][1000..]) > 0.20);
    assert!(rms(&sources[1][1000..]) < 0.002);
}
#[test]
fn resampler_preserves_filter_history_over_input_chunk_boundary_and_cleans_private_spool() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("impulse.wav");
    wav(
        &path,
        48000,
        (0..48000).map(|i| (if i == 32760 { 10000 } else { 0 }, 0)),
    );
    let out = read_sources(&path, 1000).unwrap();
    assert_eq!(out[0].len(), 16000);
    assert!(out[0][10938..10945].iter().any(|s| s.abs() > 0.07));
    assert_eq!(out[1][10941], 0.0);
    let prepared = prepare_sources(&path, 1000, dir.path()).unwrap();
    assert_eq!(prepared.samples, 16000);
    assert_eq!(prepared.paths.len(), 2);
    let spool = prepared.paths[0].parent().unwrap().to_owned();
    assert_eq!(
        std::fs::metadata(&spool).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&prepared.paths[0])
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(std::fs::metadata(&prepared.paths[0]).unwrap().len(), 64000);
    drop(prepared);
    assert!(!spool.exists());
}
#[test]
fn sparse_long_wav_exceeding_one_hour_enters_bounded_stream_without_duration_policy() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sparse-3601s.wav");
    let data = 3601u32 * 8000 * 2;
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(data + 36).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16u32.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&1u16.to_le_bytes()).unwrap();
    file.write_all(&8000u32.to_le_bytes()).unwrap();
    file.write_all(&16000u32.to_le_bytes()).unwrap();
    file.write_all(&2u16.to_le_bytes()).unwrap();
    file.write_all(&16u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data.to_le_bytes()).unwrap();
    file.set_len(data as u64 + 44).unwrap();
    drop(file);
    let mut reader = SourceReader::open(&path, 3_601_000).unwrap();
    for _ in 0..3 {
        let chunk = reader.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.len(), 1);
        assert_eq!(chunk[0].len(), 65536);
        assert!(chunk[0].iter().all(|s| *s == 0.0));
    }
    // Only the start is traversed: no claim of 1-hour inference or performance validation.
}
#[test]
fn vad_merges_sub_200ms_gap_but_never_beyond_25s() {
    assert_eq!(
        merge_spans(vec![(0, 1600), (1700, 3200), (6400, 8000)]),
        vec![(0, 3200), (6400, 8000)]
    );
    assert_eq!(merge_spans(vec![(0, 390000), (390100, 401000)]).len(), 2);
}
fn segment(source: AudioSource, start: u64, end: u64, text: &str) -> TranscriptSegment {
    TranscriptSegment {
        source,
        start_ms: start,
        end_ms: end,
        text: text.into(),
    }
}
#[test]
fn cross_source_echo_prefers_system_but_keeps_same_source_and_distinct_overlap() {
    let input = vec![
        segment(AudioSource::Mic, 0, 1000, "讨论下一步计划"),
        segment(AudioSource::System, 100, 1100, "讨论下一步计划。"),
        segment(AudioSource::Mic, 200, 1200, "我不同意这个提议"),
        segment(AudioSource::Mic, 300, 1300, "我不同意这个提议"),
    ];
    let result = deduplicate(input);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].source, AudioSource::System);
    assert!(!likely_duplicate("好", "好"));
    assert!(likely_duplicate("Hello meeting!", "hello meeting"));
    assert_eq!(
        deduplicate(vec![
            segment(AudioSource::Mic, 0, 1000, "hello meeting"),
            segment(AudioSource::System, 4000, 5000, "hello meeting")
        ])
        .len(),
        2
    );
}

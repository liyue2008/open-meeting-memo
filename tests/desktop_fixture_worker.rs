//! Test-only desktop worker protocol: synthetic WAV metadata, no ASR engine or playback.
#![cfg(feature = "asr")]
use omm::{
    adapters::asr::LocalAsrAdapter,
    core::{
        config::AppConfig,
        contracts::*,
        paths::{private_dir, write_private_new, Paths},
        store::{hash_bytes, Store},
    },
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
};
use uuid::Uuid;

fn worker(f: &Fixture) -> PathBuf {
    // The fixture host likewise copies these exact bytes and adds executable mode.
    let path = f._temp.path().join("fake-worker");
    if !path.exists() {
        write_private_new(
            &path,
            include_bytes!("../apps/desktop/src-tauri/fixtures/fake-worker.sh"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    path
}

struct Fixture {
    _temp: tempfile::TempDir,
    paths: Paths,
    request: AsrRequest,
}
impl Fixture {
    fn new(durations: &[u64]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let paths = Paths::resolve(&root.join("home"), &BTreeMap::new()).unwrap();
        paths.create_private_dirs().unwrap();
        let meeting = paths.data.join("meetings").join("synthetic meeting");
        private_dir(&meeting).unwrap();
        let mut config = AppConfig {
            auto_record: false,
            recording_consent: false,
            ..Default::default()
        };
        config.notifications.enabled = false;
        config.asr.model_version = "fixture-asr".into();
        let mut segments = vec![];
        for (index, duration) in durations.iter().enumerate() {
            // Spaces, Unicode, quotes and newline exercise actual JSON escaping, not sed parsing.
            let path = meeting.join(format!("音频 \"{index}\"\n.wav"));
            write_wav(&path, duration * 48);
            segments.push(AudioSegment {
                path: path.clone(),
                duration_ms: *duration,
                sha256: hash_bytes(&std::fs::read(path).unwrap()),
                capture: None,
            });
        }
        let mut store = Store::open(&paths).unwrap();
        store
            .enqueue(
                &SealedMeeting {
                    session_id: Uuid::new_v4(),
                    title: "Private synthetic worker fixture".into(),
                    directory: meeting,
                    segments: segments.clone(),
                    sealed_at_ms: chrono::Utc::now().timestamp_millis(),
                },
                &config.snapshot(),
                false,
            )
            .unwrap();
        let request = AsrRequest {
            source_kind: None,
            context: store
                .claim_asr(chrono::Utc::now().timestamp_millis(), 60_000)
                .unwrap()
                .unwrap()
                .context,
            segments,
        };
        Self {
            _temp: temp,
            paths,
            request,
        }
    }
    fn input(&self) -> Value {
        serde_json::to_value(AsrWorkerRequest {
            schema_version: 1,
            request: self.request.clone(),
            chunk_checkpoint_dir: None,
        })
        .unwrap()
    }
}
fn write_wav(path: &std::path::Path, frames: u64) {
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..frames {
        writer.write_sample(0i16).unwrap();
    }
    writer.finalize().unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}
async fn accepted(f: &Fixture, expected_ms: u64) {
    let before: Vec<_> = f
        .request
        .segments
        .iter()
        .map(|s| std::fs::read(&s.path).unwrap())
        .collect();
    let adapter = LocalAsrAdapter::with_paths(worker(f), f.paths.clone()).unwrap();
    let result = adapter.transcribe(f.request.clone()).await;
    // Drain even when an assertion fails; no force or test-owned timeout policy.
    adapter.drain().await.unwrap();
    let transcript = result.unwrap();
    assert_eq!(transcript.duration_ms, expected_ms);
    assert_eq!(transcript.model_version, "fixture-asr");
    assert_eq!(transcript.segments.len(), 1);
    assert_eq!(transcript.segments[0].source, AudioSource::Mic);
    assert_eq!(transcript.segments[0].start_ms, 0);
    assert_eq!(transcript.segments[0].end_ms, 900.min(expected_ms));
    assert_eq!(transcript.segments[0].text, "PRIVATE_SYNTHETIC_TRANSCRIPT");
    for (segment, bytes) in f.request.segments.iter().zip(before) {
        assert_eq!(std::fs::read(&segment.path).unwrap(), bytes);
    }
    assert!(!Store::open(&f.paths)
        .unwrap()
        .active_audio_readers()
        .unwrap());
}

#[tokio::test]
async fn desktop_fixture_worker_duration_matches_request_ranges() {
    for (durations, expected) in [
        (vec![1000], 1000),
        (vec![3000], 3000),
        (vec![750], 750),
        (vec![1000, 2000, 3000], 6000),
    ] {
        accepted(&Fixture::new(&durations), expected).await;
    }
    // Only the requested selection counts; other sealed files are not inputs.
    let mut selected = Fixture::new(&[1000, 2000, 3000]);
    selected.request.segments = vec![selected.request.segments.remove(1)];
    accepted(&selected, 2000).await;

    // Real rolling ranges subtract their nonzero frame origin and round cumulative
    // boundaries, not each duration_ms independently: 2 * 48024 frames = 2001ms.
    let mut rolling = Fixture::new(&[1000, 1000]);
    let instance = Uuid::new_v4();
    for (index, segment) in rolling.request.segments.iter_mut().enumerate() {
        write_wav(&segment.path, 48024);
        segment.sha256 = hash_bytes(&std::fs::read(&segment.path).unwrap());
        segment.capture = Some(CaptureSegmentIdentity {
            session_id: rolling.request.context.session_id,
            instance_id: instance,
            sequence: 7 + index as u32,
            offset_frames: 96000 + index as u64 * 48024,
            frame_count: 48024,
            sample_rate: 48000,
            descriptor_sha256: "a".repeat(64),
            legacy_wav: false,
        });
    }
    assert_eq!(
        audio_ranges_ms(&rolling.request.segments).unwrap(),
        vec![(0, 1000), (1000, 2001)]
    );
    accepted(&rolling, 2001).await;
}

fn raw(f: &Fixture, input: &[u8]) -> std::process::Output {
    let root = f._temp.path();
    let mut command = Command::new(worker(f));
    command.args(["--protocol", "1"]).env_clear();
    for (name, directory) in [
        ("HOME", "home"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_RUNTIME_DIR", "runtime"),
        ("TMPDIR", "tmp"),
    ] {
        let path = root.join(directory);
        private_dir(&path).unwrap();
        command.env(name, path);
    }
    let mut child = command
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let written = child.stdin.take().unwrap().write_all(input);
    let output = child.wait_with_output().unwrap();
    written.unwrap();
    output
}
#[test]
fn desktop_fixture_worker_rejects_invalid_requests_without_fallback() {
    let f = Fixture::new(&[1000]);
    let valid = raw(&f, &serde_json::to_vec_pretty(&f.input()).unwrap());
    assert!(valid.status.success());
    let event: Value = serde_json::from_slice(&valid.stdout).unwrap();
    assert_eq!(event["transcript"]["durationMs"], 1000);
    let mut cases = vec![b"not json".to_vec(), b"{}".to_vec()];
    for (pointer, value) in [
        ("/schema_version", json!(2)),
        ("/request/segments", json!([])),
        ("/request/segments/0/duration_ms", Value::Null),
        ("/request/segments/0/duration_ms", json!(0)),
        ("/request/segments/0/duration_ms", json!(-1)),
        ("/request/segments/0/duration_ms", json!(true)),
        ("/request/segments/0/duration_ms", json!(1000.5)),
        ("/request/segments/0/path", json!("")),
        ("/request/segments/0/path", Value::Null),
        ("/request/context/task_id", json!("not-a-uuid")),
        ("/request/context/attempt", Value::Null),
    ] {
        let mut input = f.input();
        *input.pointer_mut(pointer).unwrap() = value;
        cases.push(serde_json::to_vec(&input).unwrap());
    }
    let mut missing = f.input();
    missing["request"]["segments"][0]
        .as_object_mut()
        .unwrap()
        .remove("duration_ms");
    cases.push(serde_json::to_vec(&missing).unwrap());
    let mut partial = f.input();
    partial["request"]["segments"][0]["capture"] = json!({"frame_count":1});
    cases.push(serde_json::to_vec(&partial).unwrap());
    let rolling_fixture = Fixture::new(&[1000, 1000]);
    let mut rolling = rolling_fixture.input();
    let instance = Uuid::new_v4();
    for index in 0..2 {
        rolling["request"]["segments"][index]["capture"] = json!({
            "session_id":rolling_fixture.request.context.session_id,"instance_id":instance,
            "sequence":7+index,"offset_frames":96000+48000*index,"frame_count":48000,
            "sample_rate":48000,"descriptor_sha256":"a".repeat(64)
        });
    }
    let valid = raw(&f, &serde_json::to_vec(&rolling).unwrap());
    assert!(valid.status.success());
    let event: Value = serde_json::from_slice(&valid.stdout).unwrap();
    assert_eq!(event["transcript"]["durationMs"], 2000);
    for (pointer, value) in [
        ("/request/segments/1/capture/offset_frames", json!(144001)),
        ("/request/segments/1/duration_ms", json!(999)),
        ("/request/segments/1/capture", Value::Null),
        ("/request/segments/1/capture/frame_count", json!(0)),
        ("/request/segments/1/capture/sample_rate", json!(16000)),
        ("/request/segments/1/capture/sequence", json!(9)),
    ] {
        let mut input = rolling.clone();
        *input.pointer_mut(pointer).unwrap() = value;
        cases.push(serde_json::to_vec(&input).unwrap());
    }
    let mut overflow = rolling_fixture.input();
    overflow["request"]["segments"][0]["duration_ms"] = json!(u64::MAX);
    cases.push(serde_json::to_vec(&overflow).unwrap());
    for input in cases {
        let output = raw(&f, &input);
        assert!(
            !output.status.success(),
            "invalid request must fail, not return fixture success"
        );
        assert!(
            output.stdout.is_empty(),
            "invalid input must not emit a result"
        );
    }
}

#[tokio::test]
async fn desktop_fixture_worker_wrong_duration_still_fails_real_validation() {
    let f = Fixture::new(&[3000]);
    let wrong = f._temp.path().join("wrong-duration-worker");
    let event = json!({
        "type":"result","schema_version":1,"task_id":f.request.context.task_id,
        "attempt":f.request.context.attempt,"transcript":{
            "schemaVersion":1,"modelVersion":"fixture-asr","durationMs":1000,
            "segments":[{"source":"mic","startMs":0,"endMs":900,"text":"PRIVATE_SYNTHETIC_TRANSCRIPT"}]
        }
    });
    write_private_new(
        &wrong,
        format!(
            "#!/bin/sh\n/bin/cat >/dev/null\nprintf '%s\\n' '{}'\n",
            event
        )
        .as_bytes(),
    )
    .unwrap();
    std::fs::set_permissions(&wrong, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adapter = LocalAsrAdapter::with_paths(wrong, f.paths.clone()).unwrap();
    let result = adapter.transcribe(f.request.clone()).await;
    adapter.drain().await.unwrap();
    let error = result.unwrap_err();
    assert_eq!(error.code, ErrorCode::Corrupt);
    assert!(!error.retryable);
    assert_eq!(error.message, "ASR transcript metadata mismatch");
    assert!(!Store::open(&f.paths)
        .unwrap()
        .active_audio_readers()
        .unwrap());
}

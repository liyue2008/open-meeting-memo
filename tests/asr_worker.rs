#![cfg(feature = "asr")]
use omm::{
    adapters::asr::{
        models::{self, MODEL_VERSION},
        LocalAsrAdapter,
    },
    core::contracts::*,
};
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};
use uuid::Uuid;
fn request(root: &Path) -> AsrRequest {
    let audio = root.join("synthetic.wav");
    let mut w = hound::WavWriter::create(
        &audio,
        hound::WavSpec {
            channels: 2,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..160 {
        w.write_sample(1000i16).unwrap();
        w.write_sample(-1000i16).unwrap();
    }
    w.finalize().unwrap();
    let work = root.join("work");
    std::fs::create_dir(&work).unwrap();
    AsrRequest {
        context: TaskContext {
            session_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            attempt: 1,
            lease: Lease {
                token: Uuid::new_v4(),
                expires_at_ms: chrono::Utc::now().timestamp_millis() + 60_000,
            },
            meeting_dir: root.to_owned(),
            work_dir: work,
            config: ConfigSnapshot {
                asr: AsrConfig {
                    model_version: MODEL_VERSION.into(),
                    threads: 2,
                    timeout_ms: 2000,
                    ..Default::default()
                },
                ..Default::default()
            },
        },
        segments: vec![AudioSegment {
            sha256: models::sha256(&audio).unwrap(),
            path: audio,
            duration_ms: 10,
        }],
    }
}
fn script(root: &Path, body: &str) -> std::path::PathBuf {
    let path = root.join(format!("worker-{}", Uuid::new_v4()));
    std::fs::write(&path, format!("#!/bin/sh\n/bin/cat >/dev/null\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}
fn result(req: &AsrRequest) -> String {
    serde_json::to_string(&AsrWorkerEvent::Result {
        schema_version: 1,
        task_id: req.context.task_id,
        attempt: req.context.attempt,
        transcript: Transcript {
            schema_version: 1,
            model_version: MODEL_VERSION.into(),
            duration_ms: 10,
            segments: vec![TranscriptSegment {
                source: AudioSource::Mic,
                start_ms: 0,
                end_ms: 10,
                text: "synthetic fixture".into(),
            }],
        },
    })
    .unwrap()
}
#[tokio::test]
async fn fixed_argv_stdin_result_identity_and_sealed_audio_hash() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let req = request(&root);
    let path=script(&root,&format!("[ \"$#\" = 2 ] && [ \"$1\" = --protocol ] && [ \"$2\" = 1 ] || exit 9\nprintf '%s\\n' '{}'",result(&req)));
    let adapter = LocalAsrAdapter::new(path).unwrap();
    let output = adapter.transcribe(req.clone()).await.unwrap();
    assert_eq!(output.segments[0].source, AudioSource::Mic);
    let mut wrong = req.clone();
    wrong.context.attempt = 2;
    assert_eq!(
        adapter.transcribe(wrong).await.unwrap_err().code,
        ErrorCode::Corrupt
    );
    std::fs::write(&req.segments[0].path, "changed").unwrap();
    assert_eq!(
        adapter.transcribe(req).await.unwrap_err().code,
        ErrorCode::Corrupt
    );
}
#[tokio::test]
async fn process_timeout_signal_nonzero_bad_json_and_output_limits_do_not_return_success() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut req = request(&root);
    for body in [
        "echo temporary > abandoned.f32; kill -KILL $$",
        "echo secret-fixture >&2; exit 7",
        "printf 'not JSON\\n'",
        &format!("printf '%s\\n' '{}' '{}'", result(&req), result(&req)),
        "/usr/bin/head -c 16777217 /dev/zero",
    ] {
        let adapter = LocalAsrAdapter::new(script(&root, body)).unwrap();
        let error = adapter.transcribe(req.clone()).await.unwrap_err();
        assert!(!error.message.contains("secret-fixture"));
        if body.contains("kill") || body.contains("head") {
            assert_eq!(error.code, ErrorCode::Backend);
        }
        assert_eq!(std::fs::read_dir(&req.context.work_dir).unwrap().count(), 0);
    }
    req.context.config.asr.timeout_ms = 30;
    let adapter = LocalAsrAdapter::new(script(&root, "exec /bin/sleep 3")).unwrap();
    assert_eq!(
        adapter.transcribe(req).await.unwrap_err().code,
        ErrorCode::Timeout
    );
}
fn invoke_real_worker(req: &AsrRequest) -> (std::process::ExitStatus, Vec<u8>, Vec<u8>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_omm-asr-worker"))
        .args(["--protocol", "1"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = serde_json::to_vec(&AsrWorkerRequest {
        schema_version: 1,
        request: req.clone(),
    })
    .unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status, out.stdout, out.stderr)
}
#[test]
fn actual_worker_missing_library_model_and_corrupt_weights_are_structured_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut req = request(&root);
    for expected in [
        ErrorCode::Unavailable,
        ErrorCode::Unavailable,
        ErrorCode::Corrupt,
    ] {
        let (status, output, stderr) = invoke_real_worker(&req);
        assert!(!status.success());
        match serde_json::from_slice::<AsrWorkerEvent>(&output).unwrap() {
            AsrWorkerEvent::Error {
                error,
                task_id,
                attempt,
                ..
            } => {
                assert_eq!(error.code, expected);
                assert_eq!(task_id, req.context.task_id);
                assert_eq!(attempt, 1);
            }
            _ => panic!("false success"),
        }
        assert!(!String::from_utf8(stderr)
            .unwrap()
            .contains("synthetic fixture"));
        if req.context.config.asr.library_path.as_os_str().is_empty() {
            req.context.config.asr.library_path = "/bin/sh".into();
        } else {
            let cfg = &mut req.context.config.asr;
            for (name, path) in [
                ("encoder", &mut cfg.encoder_path),
                ("decoder", &mut cfg.decoder_path),
                ("tokens", &mut cfg.tokens_path),
                ("vad", &mut cfg.vad_path),
            ] {
                *path = root.join(name);
                std::fs::write(&*path, "not an official weight").unwrap();
            }
        }
    }
    assert_eq!(
        models::verify_models(&req.context.config.asr)
            .unwrap_err()
            .code,
        ErrorCode::Corrupt
    );
    let manifest: serde_json::Value = serde_json::from_str(models::MODEL_MANIFEST).unwrap();
    assert_eq!(manifest["native_version"], "1.13.2");
    assert_eq!(manifest["model_version"], MODEL_VERSION);
}
#[tokio::test]
async fn expired_lease_and_path_escape_are_rejected_before_worker_start() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let mut req = request(&root);
    let adapter = LocalAsrAdapter::new(root.join("does-not-exist")).unwrap();
    req.context.lease.expires_at_ms = 0;
    assert_eq!(
        adapter.transcribe(req.clone()).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    req.context.lease.expires_at_ms = chrono::Utc::now().timestamp_millis() + 60_000;
    req.segments[0].path = "/bin/sh".into();
    assert_eq!(
        adapter.transcribe(req).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

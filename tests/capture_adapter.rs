#![cfg(feature = "capture")]
use omm::{
    adapters::capture::{CaptureAutoStop, CapturePaths, MacCaptureAdapter},
    core::{contracts::*, paths::private_dir},
};
use serde_json::{json, Value};
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use tempfile::TempDir;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
};
use uuid::Uuid;

struct Fixture {
    _temp: TempDir,
    paths: CapturePaths,
    meeting: PathBuf,
    instance: Uuid,
    session: Uuid,
}
impl Fixture {
    fn new() -> Self {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/c");
        std::fs::create_dir_all(&base).unwrap();
        let temp = tempfile::Builder::new()
            .prefix("c")
            .rand_bytes(3)
            .tempdir_in(base)
            .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let paths = CapturePaths {
            state: root.join("s"),
            logs: root.join("l"),
            media: root.join("m"),
            ipc: root.join("i"),
        };
        for dir in [&paths.state, &paths.logs, &paths.media, &paths.ipc] {
            private_dir(dir).unwrap();
        }
        let meeting = paths.media.join("meeting");
        private_dir(&meeting).unwrap();
        Self {
            _temp: temp,
            paths,
            meeting,
            instance: Uuid::new_v4(),
            session: Uuid::new_v4(),
        }
    }
    fn adapter(&self, timeout_ms: u64) -> MacCaptureAdapter {
        MacCaptureAdapter::new(
            self.paths.clone(),
            self.instance,
            Duration::from_millis(timeout_ms),
        )
        .unwrap()
    }
    fn request(&self) -> CaptureRequest {
        CaptureRequest {
            segment_ms: 1_800_000,
            output_path: self.meeting.join("segment.wav"),
            context: TaskContext {
                session_id: self.session,
                task_id: Uuid::new_v4(),
                attempt: 1,
                lease: Lease {
                    token: Uuid::new_v4(),
                    expires_at_ms: chrono::Utc::now().timestamp_millis() + 60000,
                },
                meeting_dir: self.meeting.clone(),
                work_dir: self.paths.state.clone(),
                config: ConfigSnapshot::default(),
            },
        }
    }
    fn status(&self, recording: bool, path: Option<&PathBuf>) -> Value {
        json!({"recording": recording,"file":path.map(|p|p.to_str().unwrap()).unwrap_or(""),
            "session_id":if recording {self.session.to_string()} else {String::new()},
            "instance_id":self.instance,"captureIPCVersion":1,"sysReady":recording,"micReady":recording,"sysError":"","micError":""})
    }
    fn listener(&self) -> UnixListener {
        let path = self.paths.ipc.join("audio_daemon.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener
    }
}
fn wav(path: &PathBuf) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    for _ in 0..480 {
        writer.write_sample(128i16).unwrap();
        writer.write_sample(-128i16).unwrap();
    }
    writer.finalize().unwrap();
}
async fn read_request(stream: &mut tokio::net::UnixStream) -> Value {
    let mut bytes = vec![];
    tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn capture_start_stop_status_eof_and_real_wav_seal() {
    let f = Fixture::new();
    let listener = f.listener();
    let output = f.request().output_path;
    let idle = f.status(false, None);
    let active = f.status(true, Some(&output));
    let instance = f.instance;
    let session = f.session;
    let child = tokio::spawn(async move {
        for (index, action) in ["status", "start", "status", "status", "stop"]
            .iter()
            .enumerate()
        {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            assert_eq!(request["action"], *action);
            let response = match index {
                0 => idle.clone(),
                1 => {
                    assert_eq!(request["instance_id"], instance.to_string());
                    assert_eq!(request["session_id"], session.to_string());
                    assert_eq!(request["filename"], "segment.wav");
                    wav(&output);
                    json!({"status":"recording","file":output,"instance_id":instance,"session_id":session})
                }
                4 => {
                    json!({"status":"stopped","files":[output],"instance_id":instance,"session_id":session})
                }
                _ => active.clone(),
            };
            socket
                .write_all(&serde_json::to_vec(&response).unwrap())
                .await
                .unwrap();
        }
    });
    let adapter = f.adapter(2000);
    let status = adapter.start(f.request()).await.unwrap();
    assert!(status.recording && status.ready);
    let stopped = adapter.stop().await.unwrap();
    assert_eq!(stopped.session_id, Some(f.session));
    assert_eq!(stopped.duration_ms, 10);
    assert_eq!(stopped.segments.len(), 1);
    assert_eq!(stopped.segments[0].sha256.len(), 64);
    child.await.unwrap();
}

#[tokio::test]
async fn capture_resumed_session_refuses_duplicate_start() {
    let f = Fixture::new();
    let previous = f.meeting.join("previous.wav");
    wav(&previous);
    let listener = f.listener();
    let response = f.status(true, Some(&previous));
    let child = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert_eq!(read_request(&mut stream).await["action"], "status");
        stream
            .write_all(&serde_json::to_vec(&response).unwrap())
            .await
            .unwrap();
    });
    assert_eq!(
        f.adapter(2000).start(f.request()).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    child.await.unwrap();
}

#[tokio::test]
async fn capture_idle_reconciliation_seals_retained_multiple_segments() {
    let f = Fixture::new();
    let a = f.meeting.join("a.wav");
    let b = f.meeting.join("b.wav");
    wav(&a);
    wav(&b);
    let listener = f.listener();
    let mut idle = f.status(false, None);
    idle["session_id"] = json!(f.session);
    let response =
        json!({"status":"stopped", "instance_id":f.instance,"session_id":f.session,"files":[a,b]});
    let child = tokio::spawn(async move {
        for reply in [idle, response] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            stream
                .write_all(&serde_json::to_vec(&reply).unwrap())
                .await
                .unwrap();
        }
    });
    let stopped = f.adapter(2000).stop().await.unwrap();
    assert_eq!(stopped.duration_ms, 20);
    assert_eq!(stopped.segments.len(), 2);
    child.await.unwrap();
}

#[tokio::test]
async fn capture_windows_and_input_devices_use_original_wire_shapes() {
    let f = Fixture::new();
    let listener = f.listener();
    let child = tokio::spawn(async move {
        for (action, reply) in [
            (
                "windows",
                json!({"windows":[{"app":"Fixture","title":"Synthetic"}]}),
            ),
            (
                "audio_devices",
                json!({"input":[{"uid":"mic","name":"Synthetic input"}],"output":[],"default_input_uid":"mic"}),
            ),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert_eq!(read_request(&mut stream).await["action"], action);
            stream
                .write_all(&serde_json::to_vec(&reply).unwrap())
                .await
                .unwrap();
        }
    });
    let adapter = f.adapter(2000);
    let windows = adapter.windows().await.unwrap();
    assert_eq!(windows[0].id, 0);
    let devices = adapter.devices().await.unwrap();
    assert!(devices[0].is_default);
    assert_eq!(devices[0].id, "mic");
    child.await.unwrap();
}

#[tokio::test]
async fn capture_timeout_response_limits_errors_and_instance_conflict_are_not_success() {
    for (reply, expected) in [
        (
            b"{\"error\":\"sys_capture_not_ready\",\"sysError\":\"secret\"}".to_vec(),
            ErrorCode::Permission,
        ),
        (b"not-json secret".to_vec(), ErrorCode::Corrupt),
        (vec![b'x'; 1024 * 1024 + 1], ErrorCode::Corrupt),
        (b"{}".to_vec(), ErrorCode::Conflict),
    ] {
        let f = Fixture::new();
        let listener = f.listener();
        let child = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            let _ = stream.write_all(&reply).await;
        });
        let error = f.adapter(2000).status().await.unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!error.message.contains("secret"));
        child.await.unwrap();
    }
    let f = Fixture::new();
    let listener = f.listener();
    let child = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_request(&mut stream).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
    assert_eq!(
        f.adapter(20).status().await.unwrap_err().code,
        ErrorCode::Timeout
    );
    child.abort();
}

#[tokio::test]
async fn capture_path_and_socket_permissions_fail_before_mutation() {
    let f = Fixture::new();
    let adapter = f.adapter(2000);
    let mut request = f.request();
    request.output_path = f.paths.state.join("escape.wav");
    assert_eq!(
        adapter.start(request).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    let mut request = f.request();
    request.context.lease.expires_at_ms = 0;
    assert_eq!(
        adapter.start(request).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    std::os::unix::fs::symlink("missing", f.request().output_path).unwrap();
    assert_eq!(
        adapter.start(f.request()).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );
    let _listener = f.listener();
    std::fs::set_permissions(
        adapter.socket_path(),
        std::fs::Permissions::from_mode(0o666),
    )
    .unwrap();
    assert_eq!(
        adapter.status().await.unwrap_err().code,
        ErrorCode::Permission
    );
    let mut paths = f.paths.clone();
    paths.ipc = f.paths.ipc.join("x".repeat(104));
    private_dir(&paths.ipc).unwrap();
    assert!(MacCaptureAdapter::new(paths, f.instance, Duration::from_secs(1)).is_err());
}

#[tokio::test]
async fn capture_stop_rejects_segment_outside_media() {
    let f = Fixture::new();
    let foreign = f.paths.state.join("foreign.wav");
    wav(&foreign);
    let listener = f.listener();
    let mut status = f.status(false, None);
    status["session_id"] = json!(f.session);
    let reply = json!({"status":"stopped","session_id":f.session,"instance_id":f.instance,"files":[foreign]});
    let child = tokio::spawn(async move {
        for value in [status, reply] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            stream
                .write_all(&serde_json::to_vec(&value).unwrap())
                .await
                .unwrap();
        }
    });
    assert_eq!(
        f.adapter(2000).stop().await.unwrap_err().code,
        ErrorCode::Permission
    );
    child.await.unwrap();
}

#[test]
fn capture_callback_requires_current_recording_session_and_instance() {
    let f = Fixture::new();
    let status = CaptureStatus {
        recording: true,
        path: None,
        ready: true,
        session_id: Some(f.session),
        errors: vec![],
    };
    let mut event = CaptureAutoStop {
        schema_version: 1,
        command: "capture_auto_stop".into(),
        session_id: f.session,
        instance_id: f.instance,
    };
    event.validate(f.instance, &status).unwrap();
    event.session_id = Uuid::new_v4();
    assert_eq!(
        event.validate(f.instance, &status).unwrap_err().code,
        ErrorCode::Conflict
    );
    event.command = "other".into();
    assert_eq!(
        event.validate(f.instance, &status).unwrap_err().code,
        ErrorCode::InvalidInput
    );
}

// This child is the compiled Swift helper in synthetic fixture mode, never AppDelegate.
struct NativeChild(std::process::Child);
impl Drop for NativeChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn native_command(f: &Fixture, flag: &str) -> std::process::Command {
    let binary = std::env::var_os("OMM_CAPTURE_TEST_BINARY")
        .expect("build capture development bundle and set OMM_CAPTURE_TEST_BINARY");
    let mut command = std::process::Command::new(binary);
    command
        .arg(flag)
        .env_clear()
        .env("YULU_APPLICATION_SUPPORT_DIR", &f.paths.state)
        .env("YULU_LOG_DIR", &f.paths.logs)
        .env("YULU_MEDIA_LIBRARY_DIR", &f.paths.media)
        .env("YULU_IPC_DIR", &f.paths.ipc)
        .env("YULU_SERVICE_OWNER", f.instance.to_string())
        .env("OMM_SERVICE_SOCKET", f.paths.ipc.join("service.sock"))
        .env("OMM_TEST_ROOT", &f.paths.state)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

#[tokio::test]
#[ignore = "requires compiled Swift development bundle; runs only synthetic hooks"]
async fn capture_native_synthetic_ipc_and_silence_callback_round_trip() {
    let f = Fixture::new();
    let _child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    let adapter = f.adapter(3000);
    for _ in 0..200 {
        if adapter.socket_path().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!adapter.status().await.unwrap().recording);
    assert_eq!(
        adapter.windows().await.unwrap()[0].title,
        "Synthetic meeting"
    );
    assert!(adapter.devices().await.unwrap()[0].is_default);
    assert!(adapter.start(f.request()).await.unwrap().ready);
    let service = UnixListener::bind(f.paths.ipc.join("service.sock")).unwrap();
    std::fs::set_permissions(
        f.paths.ipc.join("service.sock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let instance = f.instance;
    let session = f.session;
    let server = tokio::spawn(async move {
        let (mut stream, _) = service.accept().await.unwrap();
        let event: CaptureAutoStop =
            serde_json::from_value(read_request(&mut stream).await).unwrap();
        assert_eq!(event.instance_id, instance);
        assert_eq!(event.session_id, session);
        assert_eq!(event.command, "capture_auto_stop");
        stream.write_all(b"{\"ok\":true}").await.unwrap();
    });
    let mut callback = tokio::process::Command::from(native_command(&f, "--callback-self-test"))
        .spawn()
        .unwrap();
    assert!(callback.wait().await.unwrap().success());
    server.await.unwrap();
    let stopped = adapter.stop().await.unwrap();
    assert_eq!(stopped.duration_ms, 1000);
    assert_eq!(stopped.segments.len(), 1);
    assert_eq!(
        adapter.stop().await.unwrap().segments[0].sha256,
        stopped.segments[0].sha256
    );
    assert!(!adapter.status().await.unwrap().recording);
}

#[tokio::test]
#[ignore = "requires compiled Swift development bundle; synthetic restart recovery only"]
async fn capture_native_restart_resumes_session_and_seals_both_segments() {
    let f = Fixture::new();
    let adapter = f.adapter(3000);
    let child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    for _ in 0..200 {
        if adapter.status().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let first = adapter.start(f.request()).await.unwrap();
    drop(child); // Force kill: original .state.json remains recording=true.
    std::fs::remove_file(adapter.socket_path()).unwrap();
    let _resumed = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    let mut current = None;
    for _ in 0..200 {
        if let Ok(status) = adapter.status().await {
            if status.recording && status.path != first.path {
                current = Some(status);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let current = current.expect("helper did not resume synthetic recording");
    assert_eq!(current.session_id, Some(f.session));
    assert_eq!(
        current.path.as_ref().unwrap().parent(),
        Some(f.meeting.as_path())
    );
    let mut request = f.request();
    request.output_path = f.meeting.join("duplicate.wav");
    assert_eq!(
        adapter.start(request).await.unwrap_err().code,
        ErrorCode::Conflict
    );
    let stopped = adapter.stop().await.unwrap();
    assert_eq!(stopped.segments.len(), 2);
    for (sequence, audio) in stopped.segments.iter().enumerate() {
        assert_eq!(audio.duration_ms, 1000);
        let descriptor: serde_json::Value = serde_json::from_slice(
            &std::fs::read(format!("{}.sealed.json", audio.path.display())).unwrap(),
        )
        .unwrap();
        assert_eq!(descriptor["sequence"], sequence);
        assert_eq!(descriptor["offset_frames"], sequence * 48000);
        assert_eq!(descriptor["frame_count"], 48000);
        assert_eq!(descriptor["temporary"], sequence == 1);
        let samples: Vec<i16> = hound::WavReader::open(&audio.path)
            .unwrap()
            .samples::<i16>()
            .map(|sample| sample.unwrap())
            .collect();
        assert_eq!(samples.len(), 96000);
        assert!(samples
            .as_chunks::<2>()
            .0
            .iter()
            .all(|pair| *pair == [9830, 512]));
    }
    assert_eq!(stopped.segments[0].path, first.path.unwrap());
    assert_eq!(stopped.segments[1].path, current.path.unwrap());
}

#[test]
#[ignore = "requires compiled Swift development bundle; no hardware capture"]
fn capture_native_original_path_pcm_recovery_and_silence_self_tests() {
    for flag in ["--self-test", "--path-contract-self-test"] {
        let f = Fixture::new();
        let status = native_command(&f, flag).status().unwrap();
        assert!(status.success(), "native {flag} failed");
    }
}

#[tokio::test]
#[ignore = "requires compiled Swift development bundle; callback errors/timeouts without recording"]
async fn capture_native_callback_rejects_bad_ack_and_times_out_without_ack() {
    for stall in [false, true] {
        let f = Fixture::new();
        std::fs::write(
            f.paths.state.join("omm-session.json"),
            serde_json::to_vec(&json!({"session_id":f.session})).unwrap(),
        )
        .unwrap();
        let socket_path = f.paths.ipc.join("service.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            if stall {
                tokio::time::sleep(Duration::from_secs(4)).await;
            } else {
                stream
                    .write_all(b"{\"ok\":false,\"error\":\"secret\"}")
                    .await
                    .unwrap();
            }
        });
        let mut callback =
            tokio::process::Command::from(native_command(&f, "--callback-self-test"))
                .spawn()
                .unwrap();
        let status = tokio::time::timeout(Duration::from_secs(3), callback.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(!status.success());
        server.abort();
    }
}

#[tokio::test]
#[ignore = "requires compiled Swift development bundle; synthetic recovery-state write failure"]
async fn capture_native_state_write_failure_is_not_start_or_stop_success() {
    let f = Fixture::new();
    std::fs::create_dir(f.paths.state.join("omm-session.json")).unwrap();
    let _child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    let adapter = f.adapter(3000);
    for _ in 0..200 {
        if adapter.status().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        adapter.start(f.request()).await.unwrap_err().code,
        ErrorCode::Io
    );
    let status = adapter.status().await.unwrap();
    assert!(!status.recording);
    assert_eq!(status.errors[0].code, ErrorCode::Io);
    assert_eq!(adapter.stop().await.unwrap_err().code, ErrorCode::Io);
}

#[tokio::test]
async fn capture_stop_rejects_truncated_wav_instead_of_sealing_success() {
    let f = Fixture::new();
    let path = f.meeting.join("truncated.wav");
    wav(&path);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(44)
        .unwrap();
    let listener = f.listener();
    let mut idle = f.status(false, None);
    idle["session_id"] = json!(f.session);
    let reply =
        json!({"status":"stopped","instance_id":f.instance,"session_id":f.session,"files":[path]});
    let child = tokio::spawn(async move {
        for value in [idle, reply] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            stream
                .write_all(&serde_json::to_vec(&value).unwrap())
                .await
                .unwrap();
        }
    });
    assert_eq!(
        f.adapter(2000).stop().await.unwrap_err().code,
        ErrorCode::Corrupt
    );
    child.await.unwrap();
}

async fn native_request(f: &Fixture, request: Value) -> Value {
    let mut stream = tokio::net::UnixStream::connect(f.paths.ipc.join("audio_daemon.sock"))
        .await
        .unwrap();
    stream
        .write_all(&serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
    let mut bytes = vec![];
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
async fn wait_native(f: &Fixture) {
    for _ in 0..200 {
        if f.adapter(3000).status().await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("synthetic helper not ready");
}
#[tokio::test]
#[ignore = "requires Swift bundle; quit protocol only, synthetic PCM"]
async fn capture_native_quit_rejects_wrong_instance_version_and_recording() {
    let f = Fixture::new();
    let mut child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    let reply = native_request(
        &f,
        json!({"action":"quit", "instance_id":Uuid::new_v4(), "captureIPCVersion":1}),
    )
    .await;
    assert_eq!(reply["error"], "capture_instance_conflict");
    assert!(child.0.try_wait().unwrap().is_none());
    let reply = native_request(
        &f,
        json!({"action":"quit", "instance_id":f.instance, "captureIPCVersion":2}),
    )
    .await;
    assert_eq!(reply["error"], "capture_protocol_conflict");
    let adapter = f.adapter(3000);
    adapter.start(f.request()).await.unwrap();
    let reply = native_request(
        &f,
        json!({"action":"quit", "instance_id":f.instance, "captureIPCVersion":1}),
    )
    .await;
    assert_eq!(reply["error"], "already_recording");
    assert!(adapter.status().await.unwrap().recording);
    assert!(child.0.try_wait().unwrap().is_none());
    adapter.stop().await.unwrap();
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn capture_shutdown_mock_ack_errors_and_exit_timeout_are_not_success() {
    for (case, expected) in [
        ("instance", ErrorCode::Conflict),
        ("version", ErrorCode::Conflict),
        ("pid", ErrorCode::Conflict),
        ("ack", ErrorCode::Corrupt),
        ("malformed", ErrorCode::Corrupt),
        ("rejected", ErrorCode::Conflict),
        ("stall", ErrorCode::Timeout),
        ("still_alive", ErrorCode::Timeout),
        ("socket_removed", ErrorCode::Timeout),
    ] {
        let f = Fixture::new();
        let listener = f.listener();
        let mut status = f.status(false, None);
        status["pid"] = json!(std::process::id());
        let instance = f.instance;
        let path = f.paths.ipc.join("audio_daemon.sock");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            assert_eq!(read_request(&mut stream).await["action"], "status");
            stream
                .write_all(&serde_json::to_vec(&status).unwrap())
                .await
                .unwrap();
            drop(stream);
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            assert_eq!(
                request,
                json!({"action":"quit","instance_id":instance,"captureIPCVersion":1})
            );
            if case == "stall" {
                tokio::time::sleep(Duration::from_secs(2)).await;
                return;
            }
            let mut ack = json!({"status":"bye","instance_id":instance,"captureIPCVersion":1,"pid":std::process::id()});
            match case {
                "instance" => ack["instance_id"] = json!(Uuid::new_v4()),
                "version" => ack["captureIPCVersion"] = json!(2),
                "pid" => ack["pid"] = json!(0),
                "ack" => ack["status"] = json!("wrong"),
                "rejected" => ack = json!({"error":"already_recording"}),
                "socket_removed" => std::fs::remove_file(path).unwrap(),
                _ => (),
            }
            let bytes = if case == "malformed" {
                b"invalid secret".to_vec()
            } else {
                serde_json::to_vec(&ack).unwrap()
            };
            stream.write_all(&bytes).await.unwrap();
            // This peer is the live test process. A valid bye (even with the
            // socket removed) must never count as process-exit evidence.
        });
        let start = std::time::Instant::now();
        let err = f.adapter(150).shutdown().await.unwrap_err();
        assert_eq!(err.code, expected, "{case}");
        assert!(!err.message.contains("secret"));
        assert!(start.elapsed() < Duration::from_secs(1), "{case}");
        if case == "stall" {
            server.abort();
        } else {
            server.await.unwrap();
        }
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn capture_shutdown_status_identity_errors_do_not_send_quit() {
    for case in ["instance", "version", "pid"] {
        let f = Fixture::new();
        let listener = f.listener();
        let mut status = f.status(false, None);
        status["pid"] = json!(std::process::id());
        match case {
            "instance" => status["instance_id"] = json!(Uuid::new_v4()),
            "version" => status["captureIPCVersion"] = json!(2),
            _ => status["pid"] = json!(0),
        }
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            stream
                .write_all(&serde_json::to_vec(&status).unwrap())
                .await
                .unwrap();
            drop(stream);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err()
            );
        });
        assert_eq!(
            f.adapter(200).shutdown().await.unwrap_err().code,
            ErrorCode::Conflict
        );
        server.await.unwrap();
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn capture_shutdown_replaced_endpoint_does_not_receive_quit_or_get_removed() {
    let f = Fixture::new();
    let listener = f.listener();
    let path = f.paths.ipc.join("audio_daemon.sock");
    let mut status = f.status(false, None);
    status["pid"] = json!(std::process::id());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_request(&mut stream).await;
        std::fs::remove_file(&path).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        stream
            .write_all(&serde_json::to_vec(&status).unwrap())
            .await
            .unwrap();
        drop(stream);
        assert!(
            tokio::time::timeout(Duration::from_millis(50), replacement.accept())
                .await
                .is_err()
        );
        assert!(path.exists());
    });
    assert_eq!(
        f.adapter(200).shutdown().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    server.await.unwrap();
    assert!(f.paths.ipc.join("audio_daemon.sock").exists());
}

#[tokio::test]
async fn capture_shutdown_without_exit_evidence_is_unavailable() {
    let f = Fixture::new();
    let adapter = f.adapter(200);
    assert_eq!(
        adapter.shutdown().await.unwrap_err().code,
        ErrorCode::Unavailable
    );
    assert_eq!(
        adapter.shutdown().await.unwrap_err().code,
        ErrorCode::Unavailable
    );
}

async fn wait_native_exit(child: &mut NativeChild) {
    let code = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(code) = child.0.try_wait().unwrap() {
                break code;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fixture process did not exit");
    assert!(code.success());
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires Swift bundle; prestarted helper adopted without child ownership"]
async fn capture_native_shutdown_adopted_instance_exits_and_repeated_shutdown_is_safe() {
    let f = Fixture::new();
    let mut child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    // The adapter neither launched nor owns this child, like service restart adoption.
    let adapter = f.adapter(3000);
    let wrong =
        MacCaptureAdapter::new(f.paths.clone(), Uuid::new_v4(), Duration::from_secs(1)).unwrap();
    assert_eq!(
        wrong.shutdown().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(child.0.try_wait().unwrap().is_none());
    adapter.start(f.request()).await.unwrap();
    assert_eq!(
        adapter.shutdown().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(adapter.status().await.unwrap().recording);
    assert!(child.0.try_wait().unwrap().is_none());
    adapter.stop().await.unwrap();
    adapter.shutdown().await.unwrap();
    wait_native_exit(&mut child).await;
    assert!(!adapter.socket_path().exists());
    adapter.shutdown().await.unwrap();
    assert_eq!(
        f.adapter(200).shutdown().await.unwrap_err().code,
        ErrorCode::Unavailable
    );
    // A new live instance/endpoint, even with the persistent UUID, is not
    // covered by this adapter's cached exit proof and must be left untouched.
    let mut replacement = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    assert_eq!(
        adapter.shutdown().await.unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(replacement.0.try_wait().unwrap().is_none());
    f.adapter(3000).shutdown().await.unwrap();
    wait_native_exit(&mut replacement).await;
}

#[tokio::test]
#[ignore = "requires Swift bundle; old instance must preserve replacement socket inode"]
async fn capture_native_quit_cleans_only_its_owned_socket_inode() {
    let f = Fixture::new();
    let mut child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    let path = f.paths.ipc.join("audio_daemon.sock");
    let mut original = tokio::net::UnixStream::connect(&path).await.unwrap();
    std::fs::remove_file(&path).unwrap();
    let replacement = f.listener();
    original
        .write_all(
            &serde_json::to_vec(
                &json!({"action":"quit","instance_id":f.instance,"captureIPCVersion":1}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    original.shutdown().await.unwrap();
    let mut bytes = vec![];
    tokio::time::timeout(Duration::from_secs(3), original.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    let ack: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(ack["status"], "bye");
    assert_eq!(ack["instance_id"], f.instance.to_string());
    assert_eq!(ack["captureIPCVersion"], 1);
    assert_eq!(ack["pid"], child.0.id());
    wait_native_exit(&mut child).await;
    assert!(path.exists());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), replacement.accept())
            .await
            .is_err()
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn capture_shutdown_exit_with_unclean_or_replaced_endpoint_is_not_success() {
    // Spawn this exact test as a controlled protocol peer in a separate process.
    // No helper, hardware, signal-based exit inference or new dependency is involved.
    if let Some(path) = std::env::var_os("OMM_CAPTURE_SHUTDOWN_MOCK_SOCKET") {
        use std::io::{Read, Write};
        let path = PathBuf::from(path);
        let instance = std::env::var("OMM_CAPTURE_SHUTDOWN_MOCK_INSTANCE").unwrap();
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        for action in ["status", "quit"] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = vec![];
            stream.read_to_end(&mut bytes).unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(request["action"], action);
            if action == "quit" {
                assert_eq!(request["instance_id"], instance);
                assert_eq!(request["captureIPCVersion"], 1);
                stream.write_all(&serde_json::to_vec(&json!({"status":"bye", "instance_id":instance, "captureIPCVersion":1,"pid":std::process::id()})).unwrap()).unwrap();
                // Replace only after ack, keeping old listener open so inode cannot
                // be immediately reused. Parent must not confuse NOTE_EXIT with
                // successful cleanup or remove this unowned replacement.
                if std::env::var("OMM_CAPTURE_SHUTDOWN_MOCK_MODE").unwrap() == "stale" {
                    return; // Exit without unlinking: a stale owned endpoint is IO failure.
                }
                std::fs::remove_file(&path).unwrap();
                let _replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                return;
            }
            stream.write_all(&serde_json::to_vec(&json!({"recording":false,"file":"","instance_id":instance,"captureIPCVersion":1,"pid":std::process::id()})).unwrap()).unwrap();
        }
        unreachable!();
    }
    for (mode, expected) in [("replaced", ErrorCode::Conflict), ("stale", ErrorCode::Io)] {
        let f = Fixture::new();
        let adapter = f.adapter(3000);
        let mut child = NativeChild(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "capture_shutdown_exit_with_unclean_or_replaced_endpoint_is_not_success",
                    "--nocapture",
                ])
                .env_clear()
                .env("OMM_CAPTURE_SHUTDOWN_MOCK_SOCKET", adapter.socket_path())
                .env("OMM_CAPTURE_SHUTDOWN_MOCK_INSTANCE", f.instance.to_string())
                .env("OMM_CAPTURE_SHUTDOWN_MOCK_MODE", mode)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        for _ in 0..200 {
            if adapter.socket_path().exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(adapter.shutdown().await.unwrap_err().code, expected);
        wait_native_exit(&mut child).await;
        assert!(adapter.socket_path().exists());
        assert_eq!(
            adapter.shutdown().await.unwrap_err().code,
            ErrorCode::Unavailable
        );
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn capture_shutdown_lock_wait_is_inside_the_total_timeout() {
    let f = Fixture::new();
    let listener = f.listener();
    let adapter = std::sync::Arc::new(f.adapter(150));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let _ = read_request(&mut stream).await;
        tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let other = adapter.clone();
    let status = tokio::spawn(async move { other.status().await });
    rx.await.unwrap();
    let start = std::time::Instant::now();
    assert_eq!(
        adapter.shutdown().await.unwrap_err().code,
        ErrorCode::Timeout
    );
    assert!(start.elapsed() < Duration::from_millis(250));
    assert_eq!(status.await.unwrap().unwrap_err().code, ErrorCode::Timeout);
    server.abort();
}

#[test]
#[ignore = "requires compiled Swift development bundle; only synthetic paired PCM"]
fn capture_native_rolling_paired_frame_payload_and_maximum_boundary() {
    let f = Fixture::new();
    assert!(native_command(&f, "--rolling-frame-self-test")
        .status()
        .unwrap()
        .success());
}

#[tokio::test]
#[ignore = "requires compiled Swift development bundle; repeated synthetic recovery only"]
async fn capture_native_rolling_repeated_restart_keeps_frame_origin_and_payload() {
    let f = Fixture::new();
    let adapter = f.adapter(3000);
    let mut child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    for _ in 0..200 {
        if adapter.status().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut request = f.request();
    request.segment_ms = 60000;
    adapter.start(request).await.unwrap();
    let bridge_path = f.paths.state.join("omm-session.json");
    let bridge: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&bridge_path).unwrap()).unwrap();
    assert_eq!(bridge["segment_ms"], 60000);
    for _ in 0..2 {
        let previous = adapter.status().await.unwrap().path;
        drop(child);
        std::fs::remove_file(adapter.socket_path()).unwrap();
        child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
        let mut resumed = false;
        for _ in 0..200 {
            if let Ok(status) = adapter.status().await {
                if status.recording && status.path != previous {
                    resumed = true;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            resumed,
            "helper did not resume the owned synthetic recording"
        );
        let bridge: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&bridge_path).unwrap()).unwrap();
        assert_eq!(bridge["segment_ms"], 60000);
    }
    let sealed = adapter.stop().await.unwrap();
    assert_eq!(sealed.segments.len(), 3);
    for (sequence, audio) in sealed.segments.iter().enumerate() {
        let doc: serde_json::Value = serde_json::from_slice(
            &std::fs::read(format!("{}.sealed.json", audio.path.display())).unwrap(),
        )
        .unwrap();
        assert_eq!(doc["sequence"], sequence);
        assert_eq!(doc["offset_frames"], sequence * 48000);
        assert_eq!(doc["frame_count"], 48000);
        assert_eq!(doc["temporary"], sequence == 2);
        let samples: Vec<i16> = hound::WavReader::open(&audio.path)
            .unwrap()
            .samples::<i16>()
            .map(|v| v.unwrap())
            .collect();
        assert_eq!(samples.len(), 96000);
        assert!(samples
            .as_chunks::<2>()
            .0
            .iter()
            .all(|pair| *pair == [9830, 512]));
    }
    assert!(!adapter.status().await.unwrap().recording);
    drop(child);
}

#[tokio::test]
#[ignore = "explicit Swift synthetic fixture; descriptor publication failure and retry"]
async fn capture_native_descriptor_publish_failure_survives_status_and_retry() {
    let f = Fixture::new();
    let _child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    let a = f.adapter(10_000);
    a.start(f.request()).await.unwrap();
    let blocked = f.meeting.join("segment.wav.sealed.json");
    std::fs::create_dir(&blocked).unwrap();
    for _ in 0..2 {
        let out = native_request(
            &f,
            json!({"action":"stop","session_id":f.session,"instance_id":f.instance}),
        )
        .await;
        assert_eq!(out["error"], "capture_recovery_state_failed", "{out}");
        let status = native_request(&f, json!({"action":"status"})).await;
        assert_eq!(status["bridgeError"], "capture_recovery_state_failed");
        assert!(f.meeting.join("segment.wav").is_file());
    }
    std::fs::remove_dir(blocked).unwrap();
    let out = native_request(
        &f,
        json!({"action":"stop","session_id":f.session,"instance_id":f.instance}),
    )
    .await;
    assert_eq!(out["status"], "stopped", "{out}");
    assert!(f.meeting.join("segment.wav.sealed.json").is_file());
    let mut request = f.request();
    let second_session = Uuid::new_v4();
    request.context.session_id = second_session;
    request.output_path = f.meeting.join("state-failure.wav");
    a.start(request).await.unwrap();
    std::fs::set_permissions(&f.paths.state, std::fs::Permissions::from_mode(0o500)).unwrap();
    let before = std::fs::read(f.paths.state.join(".state.json")).unwrap();
    let mut responses = Vec::new();
    for _ in 0..2 {
        responses.push(
            native_request(
                &f,
                json!({"action":"stop","session_id":second_session,"instance_id":f.instance}),
            )
            .await,
        );
    }
    // Restore the private root before assertions so failures cannot strand an unwritable fixture.
    std::fs::set_permissions(&f.paths.state, std::fs::Permissions::from_mode(0o700)).unwrap();
    for response in responses {
        assert_eq!(response["error"], "capture_recovery_state_failed");
    }
    assert_eq!(
        std::fs::read(f.paths.state.join(".state.json")).unwrap(),
        before
    );
    let bridge: Value =
        serde_json::from_slice(&std::fs::read(f.paths.state.join("omm-session.json")).unwrap())
            .unwrap();
    assert_eq!(bridge["recording"], true);
    assert!(f.meeting.join("state-failure.wav").is_file());
    a.stop().await.unwrap();
    assert!(!a.status().await.unwrap().recording);
}

#[tokio::test]
#[ignore = "explicit Swift synthetic rolling/current-state/restart fixture"]
async fn capture_native_multiple_rotations_recover_tail_and_replay_without_duplicates() {
    let f = Fixture::new();
    assert!(native_command(&f, "--rolling-current-state-self-test")
        .status()
        .unwrap()
        .success());
    let bridge_path = f.paths.state.join("omm-session.json");
    let mut child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    for _ in 0..2 {
        let a = f.adapter(15_000);
        let state = a.status().await.unwrap();
        assert!(state.recording);
        let bridge: Value = serde_json::from_slice(&std::fs::read(&bridge_path).unwrap()).unwrap();
        let files = bridge["files"].as_array().unwrap();
        assert!(files.len() >= 4);
        let mut frame = 0;
        for (sequence, file) in files[..files.len() - 1].iter().enumerate() {
            let descriptor: Value = serde_json::from_slice(
                &std::fs::read(format!("{}.sealed.json", file.as_str().unwrap())).unwrap(),
            )
            .unwrap();
            assert_eq!(descriptor["sequence"], sequence);
            assert_eq!(descriptor["offset_frames"], frame);
            frame += descriptor["frame_count"].as_u64().unwrap();
        }
        drop(child);
        std::fs::remove_file(a.socket_path()).unwrap();
        child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
        wait_native(&f).await;
    }
    let a = f.adapter(15_000);
    a.status().await.unwrap();
    a.stop().await.unwrap();
}

#[tokio::test]
#[ignore = "requires compiled native helper; generated bounded manifest replay"]
async fn capture_native_manifest_after_sequence_pages_reject_mutation_and_missing_sequence() {
    let f = Fixture::new();
    let session = Uuid::new_v4();
    let dir = f.paths.media.join("bounded-generated");
    omm::core::paths::private_dir(&dir).unwrap();
    let mut files = Vec::new();
    for sequence in 0..19 {
        let path = dir.join(format!("{sequence}.wav"));
        let mut w = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..48 {
            w.write_sample(sequence as i16).unwrap();
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let doc = json!({"schema_version":1,"session_id":session,"instance_id":f.instance,"sequence":sequence,"offset_frames":sequence*48,"frame_count":48,"sample_rate":48000,"path":path,"temporary":true});
        omm::core::paths::write_private_new(
            &std::path::PathBuf::from(format!("{}.sealed.json", path.display())),
            &serde_json::to_vec(&doc).unwrap(),
        )
        .unwrap();
        files.push(path);
    }
    let bridge = f.paths.state.join("omm-session.json");
    omm::core::paths::write_private_new(&bridge,&serde_json::to_vec(&json!({"session_id":session,"origin_instance_id":f.instance,"files":files,"recording":false})).unwrap()).unwrap();
    let _child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    let mut after = -1;
    let mut delivered = Vec::new();
    for (count, more) in [(8, true), (8, true), (3, false)] {
        let page=native_request(&f,json!({"action":"sealed_segments","instance_id":f.instance,"session_id":session,"after_sequence":after,"limit":8})).await;
        assert_eq!(page["segments"].as_array().unwrap().len(), count);
        assert_eq!(page["has_more"], more);
        assert!(serde_json::to_vec(&page).unwrap().len() < 131072);
        for row in page["segments"].as_array().unwrap() {
            let seq = row["document"]["sequence"].as_i64().unwrap();
            assert_eq!(seq, after + 1);
            after = seq;
            delivered.push(seq);
        }
    }
    assert_eq!(delivered, (0..19).collect::<Vec<_>>());
    let descriptor = std::path::PathBuf::from(format!("{}.sealed.json", files[0].display()));
    let original = std::fs::read(&descriptor).unwrap();
    let mut mutated: Value = serde_json::from_slice(&original).unwrap();
    mutated["offset_frames"] = json!(7);
    std::fs::write(&descriptor, serde_json::to_vec(&mutated).unwrap()).unwrap();
    let response=native_request(&f,json!({"action":"sealed_segments","instance_id":f.instance,"session_id":session,"after_sequence":-1,"limit":8})).await;
    assert_eq!(response["error"], "capture_recovery_state_failed");
    std::fs::write(&descriptor, &original).unwrap();
    std::fs::remove_file(format!("{}.sealed.json", files[8].display())).unwrap();
    let response=native_request(&f,json!({"action":"sealed_segments","instance_id":f.instance,"session_id":session,"after_sequence":7,"limit":8})).await;
    assert_eq!(response["error"], "capture_recovery_state_failed");
}

#[tokio::test]
#[ignore = "explicit Swift synthetic pre-rolling state/bridge compatibility"]
async fn capture_native_legacy_state_promotes_only_single_original_and_restart_replays() {
    let f = Fixture::new();
    let a = f.adapter(15_000);
    let original_child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    let path = a.start(f.request()).await.unwrap().path.unwrap();
    drop(original_child);
    std::fs::remove_file(a.socket_path()).unwrap();
    omm::core::paths::write_private_replace(&f.paths.state.join(".state.json"), &serde_json::to_vec(&json!({
        "version":3, "recording":true,"backend":"daemon","title":"Generated legacy fixture", "audio_path":path, "started_at":"2026-01-01T00:00:00Z"
    })).unwrap()).unwrap();
    omm::core::paths::write_private_replace(
        &f.paths.state.join("omm-session.json"),
        &serde_json::to_vec(&json!({
            "session_id":f.session,"files":[path],"recording":true
        }))
        .unwrap(),
    )
    .unwrap();
    let mut child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    let a = f.adapter(15_000);
    for _ in 0..200 {
        if a.status().await.is_ok_and(|s| s.recording) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        a.status().await.unwrap().recording,
        "state={} bridge={}",
        std::fs::read_to_string(f.paths.state.join(".state.json")).unwrap(),
        std::fs::read_to_string(f.paths.state.join("omm-session.json")).unwrap()
    );
    let original = a.sealed_segments().await.unwrap();
    assert_eq!(original.len(), 1);
    assert!(original[0].capture.as_ref().unwrap().legacy_wav);
    assert!(path.exists());
    drop(child);
    std::fs::remove_file(a.socket_path()).unwrap();
    child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    let second = f.adapter(15_000);
    for _ in 0..200 {
        if second.status().await.is_ok_and(|s| s.recording) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(second.status().await.unwrap().recording);
    let stopped = second.stop().await.unwrap();
    assert_eq!(stopped.segments.len(), 3);
    assert!(stopped.segments[0].capture.as_ref().unwrap().legacy_wav);
    assert!(stopped.segments[1..].iter().all(|segment| !segment
        .capture
        .as_ref()
        .unwrap()
        .legacy_wav));
    assert!(path.exists());
    drop(child);
}

#[tokio::test]
#[ignore = "explicit Swift synthetic empty-current crash/restart and shutdown"]
async fn capture_native_zero_frame_current_restarts_twice_without_manifest_holes() {
    use std::io::{Seek, SeekFrom, Write};
    let f = Fixture::new();
    let a = f.adapter(15_000);
    let mut child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
    wait_native(&f).await;
    a.start(f.request()).await.unwrap();
    let mut originals = Vec::new();
    for _ in 0..2 {
        let tail = a.status().await.unwrap().path.unwrap();
        drop(child);
        let mut file = std::fs::OpenOptions::new().write(true).open(&tail).unwrap();
        file.set_len(82).unwrap();
        file.seek(SeekFrom::Start(4)).unwrap();
        file.write_all(&74u32.to_le_bytes()).unwrap();
        file.seek(SeekFrom::Start(78)).unwrap();
        file.write_all(&0u32.to_le_bytes()).unwrap();
        file.sync_all().unwrap();
        originals.push(tail);
        std::fs::remove_file(a.socket_path()).unwrap();
        child = NativeChild(native_command(&f, "--ipc-fixture").spawn().unwrap());
        wait_native(&f).await;
        let status = a.status().await.unwrap();
        assert!(status.recording, "{status:?}");
        assert!(a.sealed_segments().await.unwrap().is_empty());
        let bridge: Value =
            serde_json::from_slice(&std::fs::read(f.paths.state.join("omm-session.json")).unwrap())
                .unwrap();
        assert_eq!(bridge["files"].as_array().unwrap().len(), 1);
        assert_eq!(bridge["current_sequence"], 0);
        assert_eq!(bridge["current_offset_frames"], 0);
    }
    let sealed = a.stop().await.unwrap();
    assert_eq!(sealed.segments.len(), 1);
    assert_eq!(sealed.segments[0].capture.as_ref().unwrap().sequence, 0);
    assert_eq!(
        sealed.segments[0].capture.as_ref().unwrap().frame_count,
        48000
    );
    for path in originals {
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 82);
        assert!(!std::path::PathBuf::from(format!("{}.sealed.json", path.display())).exists());
    }
    a.shutdown().await.unwrap();
    wait_native_exit(&mut child).await;
}

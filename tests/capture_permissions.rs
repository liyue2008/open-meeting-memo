#![cfg(feature = "capture")]
use omm::{
    adapters::capture::{CapturePaths, MacCaptureAdapter},
    core::paths::private_dir,
};
use serde_json::{json, Value};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
};
use uuid::Uuid;

#[tokio::test]
async fn explicit_permission_request_is_bound_and_never_starts_recording() {
    // Short canonical paths also work with Darwin's 104-byte socket bound.
    let tmp = tempfile::Builder::new()
        .prefix("omm-perm-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let paths = CapturePaths {
        state: root.join("s"),
        logs: root.join("l"),
        media: root.join("m"),
        ipc: root.join("i"),
    };
    for p in [&paths.state, &paths.logs, &paths.media, &paths.ipc] {
        private_dir(p).unwrap();
    }
    let owner = Uuid::new_v4();
    let id = Uuid::new_v4();
    let adapter = MacCaptureAdapter::new(paths.clone(), owner, Duration::from_secs(2)).unwrap();
    let sock = paths.ipc.join("audio_daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600)).unwrap();
    let fake = tokio::spawn(async move {
        for microphone in [
            "authorized",
            "denied",
            "restricted",
            "not_determined",
            "unknown",
            "wrong_owner",
            "wrong_request",
            "false_system_authorized",
            "invalid_microphone",
            "invalid_outcome",
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            stream.read_to_end(&mut bytes).await.unwrap();
            let request: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                request,
                json!({"action":"permissions_request", "instance_id":owner, "request_id":id, "captureIPCVersion":1})
            );
            let mut reply = json!({"instance_id":owner,"request_id":id,"microphone":microphone,"systemAudio":"unknown","outcome":"completed"});
            match microphone {
                "wrong_owner" => reply["instance_id"] = json!(Uuid::new_v4()),
                "wrong_request" => reply["request_id"] = json!(Uuid::new_v4()),
                "false_system_authorized" => {
                    reply["microphone"] = json!("authorized");
                    reply["systemAudio"] = json!("authorized");
                }
                "invalid_outcome" => {
                    reply["microphone"] = json!("authorized");
                    reply["outcome"] = json!("ready");
                }
                _ => (),
            }
            stream
                .write_all(serde_json::to_string(&reply).unwrap().as_bytes())
                .await
                .unwrap();
            stream.shutdown().await.unwrap();
        }
    });
    for expected in [
        "authorized",
        "denied",
        "restricted",
        "not_determined",
        "unknown",
    ] {
        let result = adapter.request_permissions(id).await.unwrap();
        assert_eq!(result.microphone, expected);
        assert_eq!(result.system_audio, "unknown");
    }
    for _ in 0..5 {
        assert!(adapter.request_permissions(id).await.is_err());
    }
    fake.await.unwrap();
    assert_eq!(std::fs::read_dir(&paths.media).unwrap().count(), 0);
}

#[test]
fn packaging_permission_declarations_and_explicit_start_are_source_bound() {
    let package = include_str!("../packaging/desktop/package.sh");
    for key in [
        "NSMicrophoneUsageDescription",
        "NSAudioCaptureUsageDescription",
        "NSScreenCaptureUsageDescription",
    ] {
        assert!(package.contains(key), "missing {key}");
    }
    assert!(package.contains("OMMDesktop.entitlements"));
    let source = include_str!("../native/yulu-capture/audio_daemon.swift");
    assert!(source.contains("AVCaptureDevice.requestAccess(for: .audio)"));
    assert!(source.contains("--permissions-only"));
    // No implicit requests on normal AppDelegate initialization.
    assert!(!source.contains("asyncAfter(deadline: .now() + 1) { ac.probePermission() }"));
}

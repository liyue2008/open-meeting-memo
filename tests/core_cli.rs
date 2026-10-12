use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;
fn invoke(t: &TempDir, args: &[&str]) -> Output {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(t.path(), fs::Permissions::from_mode(0o700)).unwrap();
    Command::new(env!("CARGO_BIN_EXE_omm"))
        .env_clear()
        .env("HOME", t.path().join("home"))
        .env("XDG_RUNTIME_DIR", t.path().canonicalize().unwrap())
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn doctor_and_init_check_are_read_only_and_do_not_claim_native_readiness() {
    let t = TempDir::new().unwrap();
    for args in [
        vec!["doctor", "--json"],
        vec!["init", "--check", "--json"],
        vec!["paths", "--json"],
    ] {
        let output = invoke(&t, &args);
        assert!(output.status.success());
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["schemaVersion"], 1);
        assert!(!t.path().join("home").exists());
    }
    let result: serde_json::Value =
        serde_json::from_slice(&invoke(&t, &["doctor", "--json"]).stdout).unwrap();
    assert_eq!(result["data"]["captureReady"], false);
    assert_eq!(result["data"]["asrReady"], false);
    assert!(result["data"]["runtimeError"].is_null());
    assert_eq!(result["data"]["runtimeSource"], "explicit_xdg");
    assert_eq!(result["data"]["serviceReady"], false);
}
#[test]
fn init_is_explicit_and_missing_service_does_not_create_recording_tasks() {
    let t = TempDir::new().unwrap();
    assert_eq!(
        invoke(&t, &["task", "list", "--json"]).status.code(),
        Some(3)
    );
    let record = invoke(&t, &["record", "start", "--json"]);
    assert_eq!(record.status.code(), Some(6));
    let unavailable: serde_json::Value = serde_json::from_slice(&record.stdout).unwrap();
    assert_eq!(unavailable["error"]["code"], "conflict");
    let output = invoke(&t, &["init", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let cfg = t.path().join("home/.config/omm/config.json");
    assert!(cfg.exists());
    let value: serde_json::Value = serde_json::from_slice(&fs::read(cfg).unwrap()).unwrap();
    assert_eq!(value["version"], "1");
    let output = invoke(&t, &["config", "validate", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(
        invoke(&t, &["task", "list", "--json"]).status.code(),
        Some(0)
    );
}
#[test]
fn invalid_cli_arguments_have_structured_sanitized_errors() {
    let t = TempDir::new().unwrap();
    let output = invoke(&t, &["unknown-SECRET_FIXTURE", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "invalid_input");
    assert!(!String::from_utf8(output.stdout)
        .unwrap()
        .contains("SECRET_FIXTURE"));
}

#[test]
fn native_recording_commands_fail_without_creating_user_directories() {
    let t = TempDir::new().unwrap();
    let output = invoke(&t, &["record", "start", "--title", "synthetic", "--json"]);
    assert_eq!(output.status.code(), Some(6));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "conflict");
    assert!(!t.path().join("home").exists());
}

#[test]
fn wait_timeout_does_not_cancel_or_modify_persistent_task() {
    use omm::core::{
        contracts::*,
        paths::Paths,
        store::{hash_bytes, Store},
    };
    use std::collections::BTreeMap;
    let t = TempDir::new().unwrap();
    assert!(invoke(&t, &["init", "--json"]).status.success());
    let paths = Paths::resolve(&t.path().join("home"), &BTreeMap::new()).unwrap();
    let id = uuid::Uuid::new_v4();
    let directory = paths.data.join("meetings").join(id.to_string());
    omm::core::paths::private_dir(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let path = directory.join("segment.wav");
    fs::write(&path, b"synthetic").unwrap();
    let meeting = SealedMeeting {
        session_id: id,
        title: "synthetic".into(),
        directory,
        segments: vec![AudioSegment {
            path,
            duration_ms: 1000,
            sha256: hash_bytes(b"synthetic"),
        }],
        sealed_at_ms: 0,
    };
    let mut store = Store::open(&paths).unwrap();
    let task = store
        .enqueue(&meeting, &ConfigSnapshot::default(), false)
        .unwrap();
    let output = invoke(
        &t,
        &[
            "task",
            "wait",
            &task.id.to_string(),
            "--timeout",
            "0s",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(5));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["code"], "timeout");
    assert_eq!(store.task(task.id).unwrap().status, TaskStatus::Queued);
    assert_eq!(store.tasks().unwrap().len(), 1);
}

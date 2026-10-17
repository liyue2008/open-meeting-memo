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
        .env("HOME", t.path().canonicalize().unwrap().join("home"))
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
    let paths = Paths::resolve(
        &t.path().canonicalize().unwrap().join("home"),
        &BTreeMap::new(),
    )
    .unwrap();
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
            capture: None,
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
#[test]
fn codec_check_is_read_only_and_install_rejects_unknown_bundle_before_creating_state() {
    let t = TempDir::new().unwrap();
    let output = invoke(&t, &["codec", "check", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["ready"], false);
    assert!(!t.path().join("home").exists());
    let source = t.path().canonicalize().unwrap();
    let output = invoke(
        &t,
        &[
            "codec",
            "install",
            "--from",
            source.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!output.status.success());
    assert!(!t.path().join("home").exists());
}
#[test]
#[ignore = "explicit locked source-built codec bundle, synthetic isolated HOME only"]
fn codec_install_copies_verified_sources_licenses_and_backs_up_only_nonsecret_metadata() {
    let manifest = std::path::PathBuf::from(std::env::var_os("OMM_CODEC_TEST_MANIFEST").unwrap())
        .canonicalize()
        .unwrap();
    let source = manifest.parent().unwrap();
    let t = TempDir::new().unwrap();
    for _ in 0..2 {
        let output = invoke(
            &t,
            &[
                "codec",
                "install",
                "--from",
                source.to_str().unwrap(),
                "--json",
            ],
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let root = t.path().join("home/.local/share/omm/audio-toolchain");
    for file in [
        "ffmpeg",
        "ffprobe",
        "sources/ffmpeg-8.1.2.tar.xz",
        "sources/opus-1.5.2.tar.gz",
        "licenses/ffmpeg.txt",
        "licenses/opus.txt",
        "sources.json",
        "build-dev.sh",
        "manifest.json",
    ] {
        assert!(root.join(file).is_file());
    }
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(root.join("ffmpeg"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(root.join("manifest.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let output = invoke(&t, &["codec", "check", "--json"]);
    assert_eq!(output.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["ready"], true);
    assert!(t
        .path()
        .join("home/.local/state/omm/audio-toolchain-backups")
        .exists());
}

#[test]
fn legacy_one_hour_segment_config_is_clamped_in_memory_and_cli_does_not_write_it() {
    use omm::core::{config::AppConfig, paths::Paths};
    let t = TempDir::new().unwrap();
    invoke(&t, &["init", "--json"]);
    let p = Paths::resolve(
        &t.path().canonicalize().unwrap().join("home"),
        &Default::default(),
    )
    .unwrap();
    let mut cfg = AppConfig::load(&p).unwrap();
    cfg.segment_ms = 3_600_000;
    cfg.save(&p).unwrap();
    let before = fs::read(p.config.join("config.json")).unwrap();
    let loaded = AppConfig::load(&p).unwrap();
    assert_eq!(loaded.segment_ms, 1_800_000);
    loaded.validate_policy().unwrap();
    let out = invoke(&t, &["config", "show", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["segmentMs"], 1_800_000);
    assert!(String::from_utf8_lossy(&out.stderr).contains("legacy segment_ms"));
    assert_eq!(fs::read(p.config.join("config.json")).unwrap(), before);
    for value in ["-1", "59999", "1800001", "3600000", "18446744073709551616"] {
        assert!(
            !invoke(&t, &["config", "set", "segment_ms", value, "--json"])
                .status
                .success()
        );
    }
}

#[test]
fn meeting_read_uses_checkpoint_before_final_commit_but_never_masks_corrupt_final() {
    use omm::core::{
        contracts::*,
        paths::{private_dir, Paths},
        store::{hash_file, Store},
    };
    let t = TempDir::new().unwrap();
    invoke(&t, &["init", "--json"]);
    let p = Paths::resolve(
        &t.path().canonicalize().unwrap().join("home"),
        &Default::default(),
    )
    .unwrap();
    let id = uuid::Uuid::new_v4();
    let dir = p.data.join("meetings").join(id.to_string());
    private_dir(&dir).unwrap();
    let mut m = SealedMeeting {
        session_id: id,
        title: "synthetic".into(),
        directory: dir.canonicalize().unwrap(),
        segments: vec![],
        sealed_at_ms: 0,
    };
    let mut s = Store::open(&p).unwrap();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.model_version = "fixture-asr".into();
    let task = s.begin_segmented(&m, &cfg).unwrap();
    let read = || {
        invoke(
            &t,
            &[
                "meeting",
                "read",
                &id.to_string(),
                "--artifact",
                "transcript",
                "--json",
            ],
        )
    };
    let out = read();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["partial"], true);
    let path = m.directory.join("segment.wav");
    let mut wav = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..4800 {
        wav.write_sample(12i16).unwrap();
        wav.write_sample(-12i16).unwrap();
    }
    wav.finalize().unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let a = AudioSegment {
        capture: None,
        path,
        sha256: hash_file(&m.directory.join("segment.wav")).unwrap(),
        duration_ms: 100,
    };
    s.admit_segment(id, 0, &a).unwrap();
    let c = s.claim_segment(100, 1000).unwrap().unwrap();
    let tr = Transcript {
        schema_version: 1,
        model_version: cfg.asr.model_version.clone(),
        duration_ms: 100,
        segments: vec![],
    };
    s.commit_segment(&c, &tr, 101).unwrap();
    let second_path = m.directory.join("second.wav");
    fs::copy(&a.path, &second_path).unwrap();
    let second = AudioSegment {
        path: second_path,
        ..a.clone()
    };
    s.admit_segment(id, 1, &second).unwrap();
    for sealed in [false, true] {
        if sealed {
            m.segments = vec![a.clone(), second.clone()];
            m.sealed_at_ms = 102;
            s.seal_segmented(&m).unwrap();
        }
        let out = read();
        assert!(out.status.success());
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["data"]["partial"], true);
        let tr: Transcript = serde_json::from_str(v["data"]["content"].as_str().unwrap()).unwrap();
        // The partial clock spans both admitted files; only the first checkpoint is committed.
        assert_eq!(tr.duration_ms, 200);
        assert_eq!(
            s.segment_progress(id)
                .unwrap()
                .iter()
                .filter(|segment| segment.status == "done")
                .count(),
            1
        );
    }
    let pending = s.claim_segment(103, 1000).unwrap().unwrap();
    s.commit_segment(&pending, &tr, 104).unwrap();
    let c = s.claim(200, 1000).unwrap().unwrap();
    let final_transcript = Transcript {
        duration_ms: 200,
        ..tr.clone()
    };
    s.commit_transcript(&c, &final_transcript, 201).unwrap();
    assert!(read().status.success());
    fs::write(
        s.artifact_path(task.id, Stage::Transcribe).unwrap(),
        b"damaged",
    )
    .unwrap();
    let out = read();
    assert!(!out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["error"]["code"], "corrupt");
}

#[test]
fn opus_bitrate_config_is_bounded_default_48_and_cli_round_trips_without_secrets() {
    let mut cfg = omm::core::config::AppConfig::default();
    assert_eq!(cfg.audio.opus_bitrate_kbps, 48);
    let mut legacy = serde_json::to_value(&cfg).unwrap();
    legacy.as_object_mut().unwrap().remove("audio");
    let legacy: omm::core::config::AppConfig = serde_json::from_value(legacy).unwrap();
    assert_eq!(legacy.audio.opus_bitrate_kbps, 48);
    let t = TempDir::new().unwrap();
    assert!(invoke(&t, &["init"]).status.success());
    assert!(
        invoke(&t, &["config", "set", "audio.opus_bitrate_kbps", "64"])
            .status
            .success()
    );
    let show: serde_json::Value =
        serde_json::from_slice(&invoke(&t, &["config", "show", "--json"]).stdout).unwrap();
    assert_eq!(show["data"]["audio"]["opus_bitrate_kbps"], 64);
    cfg.set("audio.opus_bitrate_kbps", "64").unwrap();
    for invalid in ["0", "31", "33", "97", "-1", "18446744073709551616"] {
        assert!(
            cfg.set("audio.opus_bitrate_kbps", invalid).is_err(),
            "{invalid}"
        );
    }
}

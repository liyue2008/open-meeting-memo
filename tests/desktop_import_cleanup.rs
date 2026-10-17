//! Private synthetic audio only. No capture, playback, notifier or model processes.
use omm::{
    application::{imports, media, profile},
    core::{
        config::AppConfig,
        paths::*,
        store::{hash_file, Store},
    },
};
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
};
use uuid::Uuid;
struct Fixture {
    _temp: tempfile::TempDir,
    paths: Paths,
    owner: profile::ProfileOwner,
    input: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("omm-import-cleanup-")
            .tempdir_in("/private/tmp")
            .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let paths = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
        paths.create_private_dirs().unwrap();
        let mut cfg = AppConfig::default();
        cfg.notifications.enabled = false;
        cfg.save(&paths).unwrap();
        Store::open(&paths).unwrap();
        let owner = profile::ProfileOwner {
            schema_version: 1,
            min_reader_version: 1,
            generation: Uuid::new_v4(),
            profile_id: profile::profile_id(&paths).unwrap(),
        };
        let input = root.join("synthetic.wav");
        write_private_new(&input, &media::silent_wav(1)).unwrap();
        Self {
            _temp: temp,
            paths,
            owner,
            input,
        }
    }
    fn stage(&self, handle: &str) -> PathBuf {
        self.paths.cache.join("desktop-imports").join(handle)
    }
    async fn select(&self) -> String {
        imports::select(&self.paths, &self.owner, &self.input)
            .await
            .unwrap()["selection"]["selectionHandle"]
            .as_str()
            .unwrap()
            .into()
    }
    fn request(&self, h: &str) -> Value {
        json!({"schemaVersion":1,"requestId":Uuid::new_v4(),"payload":{"selectionHandle":h,"metadata":{"title":"Synthetic","note":"","tags":[]},"startedAtMs":1700000000000i64,"timeZone":"UTC"}})
    }
    fn expire(&self, h: &str) {
        let p = self.stage(h).join("selection.json");
        let mut v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        v["expiresAtMs"] = json!(0);
        write_private_replace(&p, &serde_json::to_vec(&v).unwrap()).unwrap();
    }
}
#[tokio::test]
async fn desktop_import_cleanup_invalid_and_sample_decode_failure_leave_no_audio() {
    let f = Fixture::new();
    fs::write(&f.input, b"not an audio file").unwrap();
    assert!(imports::select(&f.paths, &f.owner, &f.input).await.is_err());
    assert_eq!(
        fs::read_dir(f.paths.cache.join("desktop-imports"))
            .unwrap()
            .count(),
        0,
        "invalid audio staging retained"
    );
    let mut writer = hound::WavWriter::create(
        &f.input,
        hound::WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    for _ in 0..8000 {
        writer.write_sample(f32::NAN).unwrap();
    }
    writer.finalize().unwrap();
    assert!(imports::select(&f.paths, &f.owner, &f.input).await.is_err());
    assert_eq!(
        fs::read_dir(f.paths.cache.join("desktop-imports"))
            .unwrap()
            .count(),
        0,
        "failed sample decode staging retained"
    );
}
#[tokio::test]
async fn desktop_import_cleanup_commit_retries_after_reconstruction_preserve_final_audio() {
    let f = Fixture::new();
    let h = f.select().await;
    let req = f.request(&h);
    let result = imports::commit(&f.paths, &f.owner, &req).unwrap();
    assert!(!f.stage(&h).exists(), "successful commit retains staging");
    let id = Uuid::parse_str(result["meeting"]["meetingId"].as_str().unwrap()).unwrap();
    let store = Store::open(&f.paths).unwrap();
    let path = store.meeting(id).unwrap().unwrap().segments[0].path.clone();
    let hash = hash_file(&path).unwrap();
    let count = store.tasks().unwrap().len();
    drop(store);
    let paths = Paths { ..f.paths.clone() };
    assert_eq!(imports::commit(&paths, &f.owner, &req).unwrap(), result);
    let replay = f._temp.path().join("replay.json");
    write_private_new(
        &replay,
        &serde_json::to_vec(&json!({"paths":paths,"owner":f.owner,"request":req})).unwrap(),
    )
    .unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "desktop_import_commit_restart_child",
            "--nocapture",
        ])
        .env_clear()
        .env("HOME", f._temp.path())
        .env("PATH", "/usr/bin:/bin")
        .env("OMM_IMPORT_REPLAY_ARGS", &replay)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("owned import replay wait failed: {error}");
            }
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned import replay child exceeded deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(status.success());
    eprintln!(
        "desktop_import_owned_child_receipt={}",
        json!({"kind":"fresh_process_replay","pid":child.id(),"waited":true,"exitCode":status.code(),"executableSha256":hash_file(&std::env::current_exe().unwrap()).unwrap()})
    );
    let restarted: Value =
        serde_json::from_slice(&fs::read(f._temp.path().join("replayed.json")).unwrap()).unwrap();
    assert_eq!(
        restarted, result,
        "fresh process must replay the persisted commit after staging release"
    );
    assert_eq!(Store::open(&paths).unwrap().tasks().unwrap().len(), count);
    assert_eq!(hash_file(&path).unwrap(), hash);
    imports::cancel(&paths, &f.owner, &h).unwrap();
    assert_eq!(hash_file(&path).unwrap(), hash);
    let h2 = f.select().await;
    let duplicate = imports::commit(&paths, &f.owner, &f.request(&h2)).unwrap();
    assert_eq!(duplicate["duplicate"], true);
    assert!(!f.stage(&h2).exists());
    assert_eq!(hash_file(&path).unwrap(), hash);
}
#[test]
fn desktop_import_commit_restart_child() {
    let Ok(path) = std::env::var("OMM_IMPORT_REPLAY_ARGS") else {
        return;
    };
    let path = PathBuf::from(path);
    assert_eq!(path.canonicalize().unwrap(), path);
    assert!(path.starts_with("/private/tmp"));
    validate_private_file(&path).unwrap();
    let args: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let paths: Paths = serde_json::from_value(args["paths"].clone()).unwrap();
    for root in [&paths.data, &paths.state, &paths.config, &paths.cache] {
        assert!(root.starts_with(path.parent().unwrap()));
    }
    let owner: profile::ProfileOwner = serde_json::from_value(args["owner"].clone()).unwrap();
    let result = imports::commit(&paths, &owner, &args["request"]).unwrap();
    write_private_new(
        &path.parent().unwrap().join("replayed.json"),
        &serde_json::to_vec(&result).unwrap(),
    )
    .unwrap();
}
#[tokio::test]
async fn desktop_import_cleanup_cancel_accepts_expired_and_changed_generation() {
    let f = Fixture::new();
    let h = f.select().await;
    f.expire(&h);
    assert!(imports::load(&f.paths, &f.owner, &h).is_err());
    imports::cancel(&f.paths, &f.owner, &h).unwrap();
    assert!(!f.stage(&h).exists());
    let h = f.select().await;
    let changed = profile::ProfileOwner {
        generation: Uuid::new_v4(),
        ..f.owner.clone()
    };
    assert!(imports::load(&f.paths, &changed, &h).is_err());
    imports::cancel(&f.paths, &changed, &h).unwrap();
    assert!(!f.stage(&h).exists());
}
#[tokio::test]
async fn desktop_import_cleanup_refuses_unknown_contents_links_and_foreign_profile() {
    let f = Fixture::new();
    let h = f.select().await;
    let unknown = f.stage(&h).join("unknown");
    write_private_new(&unknown, b"retain me").unwrap();
    assert!(imports::cancel(&f.paths, &f.owner, &h).is_err());
    assert!(
        f.stage(&h).join("imported-mix.wav").exists(),
        "must validate entire directory before deleting anything"
    );
    fs::remove_file(unknown).unwrap();
    let mix = f.stage(&h).join("imported-mix.wav");
    fs::remove_file(&mix).unwrap();
    symlink(&f.input, &mix).unwrap();
    assert!(imports::cancel(&f.paths, &f.owner, &h).is_err());
    assert!(f.input.exists());
    fs::remove_file(&mix).unwrap();
    write_private_new(&mix, &media::silent_wav(1)).unwrap();
    let alias = f._temp.path().join("hardlinked-staging.wav");
    fs::hard_link(&mix, &alias).unwrap();
    assert!(imports::cancel(&f.paths, &f.owner, &h).is_err());
    assert!(
        mix.exists() && alias.exists(),
        "linked content must be retained"
    );
    fs::remove_file(alias).unwrap();
    let foreign = profile::ProfileOwner {
        profile_id: "0".repeat(64),
        ..f.owner.clone()
    };
    assert!(imports::cancel(&f.paths, &foreign, &h).is_err());
    assert!(mix.exists());
    fs::set_permissions(&mix, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(imports::cancel(&f.paths, &f.owner, &h).is_err());
    fs::set_permissions(&mix, fs::Permissions::from_mode(0o600)).unwrap();
    imports::cancel(&f.paths, &f.owner, &h).unwrap();
}
#[tokio::test]
async fn desktop_import_cleanup_reaps_only_owned_expired_and_recovers_interrupted_promotion() {
    let f = Fixture::new();
    let expired = f.select().await;
    f.expire(&expired);
    let live = f.select().await;
    // Selecting another file already reclaims expired selections.
    assert!(!f.stage(&expired).exists());
    let unknown = "a".repeat(32);
    private_dir(&f.stage(&unknown)).unwrap();
    write_private_new(&f.stage(&unknown).join("unknown"), b"retain").unwrap();
    let report = imports::reap_expired(
        &f.paths,
        &f.owner,
        chrono::Utc::now().timestamp_millis() + 900001,
    )
    .unwrap();
    assert_eq!(report["removed"], 1);
    assert_eq!(report["retained"], 1);
    assert!(!f.stage(&live).exists());
    assert!(f.stage(&unknown).join("unknown").exists());
    let h = f.select().await;
    let selection = imports::load(&f.paths, &f.owner, &h).unwrap();
    let dir = f
        .paths
        .data
        .join("meetings/imports")
        .join(selection.meeting_id.to_string());
    private_dir(&dir).unwrap();
    write_private_new(&dir.join("import-promotion.json"),&serde_json::to_vec(&json!({"version":1,"profileId":f.owner.profile_id,"handle":selection.handle,"meetingId":selection.meeting_id,"mixHash":selection.mix_hash})).unwrap()).unwrap();
    write_private_new(&dir.join(".import-copy"), b"interrupted copy").unwrap();
    let request = f.request(&h);
    let result = imports::commit(&f.paths, &f.owner, &request).unwrap();
    assert!(!dir.join(".import-copy").exists());
    assert_eq!(
        hash_file(&dir.join("imported-mix.wav")).unwrap(),
        selection.mix_hash
    );
    assert_eq!(
        imports::commit(&f.paths, &f.owner, &request).unwrap(),
        result
    );
}
#[tokio::test]
async fn desktop_import_cleanup_recovers_publication_link_boundary_and_retains_unknown_target() {
    let f = Fixture::new();
    let h = f.select().await;
    let selection = imports::load(&f.paths, &f.owner, &h).unwrap();
    let dir = f
        .paths
        .data
        .join("meetings/imports")
        .join(selection.meeting_id.to_string());
    private_dir(&dir).unwrap();
    write_private_new(&dir.join("import-promotion.json"),&serde_json::to_vec(&json!({"version":1,"profileId":f.owner.profile_id,"handle":selection.handle,"meetingId":selection.meeting_id,"mixHash":selection.mix_hash})).unwrap()).unwrap();
    write_private_new(
        &dir.join(".import-copy"),
        &fs::read(f.stage(&h).join("imported-mix.wav")).unwrap(),
    )
    .unwrap();
    fs::hard_link(dir.join(".import-copy"), dir.join("imported-mix.wav")).unwrap();
    imports::commit(&f.paths, &f.owner, &f.request(&h)).unwrap();
    assert!(!dir.join(".import-copy").exists());
    let h = f.select().await;
    let selection = imports::load(&f.paths, &f.owner, &h).unwrap();
    let dir = f
        .paths
        .data
        .join("meetings/imports")
        .join(selection.meeting_id.to_string());
    private_dir(&dir).unwrap();
    write_private_new(&dir.join("imported-mix.wav"), b"unknown destination").unwrap();
    // A distinct source bypasses content dedup, without touching the unrelated target.
    let source = f.stage(&h).join("selection.json");
    let mut selection: Value = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    selection["sourceHash"] = json!("e".repeat(64));
    write_private_replace(&source, &serde_json::to_vec(&selection).unwrap()).unwrap();
    assert!(imports::commit(&f.paths, &f.owner, &f.request(&h)).is_err());
    assert_eq!(
        fs::read(dir.join("imported-mix.wav")).unwrap(),
        b"unknown destination"
    );
}

#[tokio::test]
async fn desktop_import_decoder_failure_waits_owned_child_and_cleans_known_output() {
    let f = Fixture::new();
    let root = f.paths.data.join("audio-toolchain");
    private_dir(&root).unwrap();
    let pid_file = root.join("owned-decoder.pid");
    let decoder = root.join("ffmpeg");
    let script=format!("#!/bin/sh\nfor last do :; done\nprintf '%s' \"$$\" > '{}'\nprintf 'partial decoder output' > \"$last\"\nexit 7\n",pid_file.display());
    write_private_new(&decoder, script.as_bytes()).unwrap();
    fs::set_permissions(&decoder, fs::Permissions::from_mode(0o700)).unwrap();
    let manifest = json!({"schemaVersion":1,"ffmpegVersion":"8.1.2","opusVersion":"1.5.2","license":"LGPL-2.1-or-later","gplEnabled":false,"networkEnabled":false,"sourceManifestSha256":omm::core::store::hash_bytes(include_bytes!("../packaging/audio/sources.json")),"ffmpegSha256":hash_file(&decoder).unwrap(),"ffprobeSha256":"0".repeat(64),"signing":"ad-hoc-development"});
    write_private_new(
        &root.join("manifest.json"),
        &serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    // Just enough self-generated framing for the bounded metadata parser; no real decoder/media.
    let source = f.paths.data.join("synthetic.opus");
    let mut page = vec![0u8; 47];
    page[..4].copy_from_slice(b"OggS");
    page[6..14].copy_from_slice(&4800u64.to_le_bytes());
    page[26] = 1;
    page[27] = 19;
    page[28..36].copy_from_slice(b"OpusHead");
    page[36] = 1;
    page[37] = 1;
    write_private_new(&source, &page).unwrap();
    let error = imports::select(&f.paths, &f.owner, &source)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "audio codec failed; diagnostics withheld"
    );
    let pid = fs::read_to_string(pid_file)
        .unwrap()
        .parse::<i32>()
        .unwrap();
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "owned failed decoder is still live"
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    assert_eq!(
        fs::read_dir(f.paths.cache.join("desktop-imports"))
            .unwrap()
            .count(),
        0,
        "failed decoder left imported staging behind"
    );
    assert!(source.exists(), "the selected source must never be removed");
    eprintln!(
        "desktop_import_owned_child_receipt={}",
        json!({"kind":"failed_owned_decoder","pid":pid,"drained":true,"esrchConfirmed":true,"executableSha256":hash_file(&decoder).unwrap()})
    );
}

// Recreate the exact on-disk boundaries before SQLite admission, not a UI cancellation.
fn interrupted_promotion(f: &Fixture, h: &str, boundary: &str) -> PathBuf {
    let s = imports::load(&f.paths, &f.owner, h).unwrap();
    let dir = f
        .paths
        .data
        .join("meetings/imports")
        .join(s.meeting_id.to_string());
    private_dir(&dir).unwrap();
    write_private_new(
        &dir.join("import-promotion.json"),
        &serde_json::to_vec(&json!({
            "version":1,"profileId":f.owner.profile_id,"handle":s.handle,
            "meetingId":s.meeting_id,"mixHash":s.mix_hash
        }))
        .unwrap(),
    )
    .unwrap();
    let bytes = fs::read(f.stage(h).join("imported-mix.wav")).unwrap();
    match boundary {
        "marker" => {}
        "partial" => {
            write_private_new(&dir.join(".import-copy"), &bytes[..bytes.len() / 2]).unwrap()
        }
        "copied" => write_private_new(&dir.join(".import-copy"), &bytes).unwrap(),
        "linked" => {
            write_private_new(&dir.join(".import-copy"), &bytes).unwrap();
            fs::hard_link(dir.join(".import-copy"), dir.join("imported-mix.wav")).unwrap();
        }
        "published" => write_private_new(&dir.join("imported-mix.wav"), &bytes).unwrap(),
        _ => panic!("unknown synthetic interruption boundary"),
    }
    assert!(Store::open_read_only(&f.paths)
        .unwrap()
        .meeting(s.meeting_id)
        .unwrap()
        .is_none());
    dir
}
#[tokio::test]
async fn desktop_import_interrupted_abandon_cancel_reclaims_only_uncommitted_promotion() {
    for boundary in ["partial", "copied", "linked", "published", "marker"] {
        let f = Fixture::new();
        let h = f.select().await;
        let dir = interrupted_promotion(&f, &h, boundary);
        imports::cancel(&f.paths, &f.owner, &h).unwrap();
        assert!(
            !dir.exists(),
            "cancel orphaned {boundary} promotion audio/record"
        );
        assert!(!f.stage(&h).exists());
        assert!(f.input.exists());
    }
}
#[tokio::test]
async fn desktop_import_interrupted_abandon_expiry_reclaims_only_uncommitted_promotion() {
    for boundary in ["partial", "copied", "linked", "published", "marker"] {
        let f = Fixture::new();
        let h = f.select().await;
        let dir = interrupted_promotion(&f, &h, boundary);
        f.expire(&h);
        let report =
            imports::reap_expired(&f.paths, &f.owner, chrono::Utc::now().timestamp_millis())
                .unwrap();
        assert_eq!(report["removed"], 1);
        assert_eq!(report["retained"], 0);
        assert!(
            !dir.exists(),
            "expiry orphaned {boundary} promotion audio/record"
        );
        assert!(!f.stage(&h).exists());
        assert!(f.input.exists());
    }
}

fn tree_snapshot(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, String> {
    use std::os::unix::fs::MetadataExt;
    fn visit(
        root: &std::path::Path,
        p: &std::path::Path,
        out: &mut std::collections::BTreeMap<PathBuf, String>,
    ) {
        let m = fs::symlink_metadata(p).unwrap();
        let content = if m.is_symlink() {
            format!("link:{}", fs::read_link(p).unwrap().display())
        } else if m.is_file() {
            hash_file(p).unwrap()
        } else {
            "directory".into()
        };
        out.insert(
            p.strip_prefix(root).unwrap().into(),
            format!("{}:{}:{}:{content}", m.mode(), m.uid(), m.nlink()),
        );
        if m.is_dir() {
            for entry in fs::read_dir(p).unwrap() {
                visit(root, &entry.unwrap().path(), out);
            }
        }
    }
    let mut out = Default::default();
    visit(root, root, &mut out);
    out
}
#[tokio::test]
async fn desktop_import_interrupted_abandon_retains_every_unknown_or_unsafe_object() {
    for defect in [
        "record_profile",
        "record_handle",
        "record_meeting",
        "record_hash",
        "record_extra",
        "record_version",
        "record_corrupt",
        "record_missing",
        "record_oversize",
        "record_link",
        "unknown_file",
        "unknown_dir",
        "target_hash",
        "partial_bytes",
        "target_link",
        "external_hardlink",
        "unrelated_pair",
        "mode",
        "directory_mode",
        "directory_link",
        "source_hash",
        "selection_missing",
    ] {
        let f = Fixture::new();
        let h = f.select().await;
        let dir = interrupted_promotion(
            &f,
            &h,
            if defect == "partial_bytes" {
                "partial"
            } else {
                "published"
            },
        );
        let marker = dir.join("import-promotion.json");
        let target = dir.join("imported-mix.wav");
        if defect.starts_with("record_")
            && ![
                "record_corrupt",
                "record_missing",
                "record_oversize",
                "record_link",
            ]
            .contains(&defect)
        {
            let mut record: Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
            match defect {
                "record_profile" => record["profileId"] = json!("0".repeat(64)),
                "record_handle" => record["handle"] = json!("0".repeat(32)),
                "record_meeting" => record["meetingId"] = json!(Uuid::new_v4()),
                "record_hash" => record["mixHash"] = json!("0".repeat(64)),
                "record_extra" => record["unknown"] = json!(true),
                "record_version" => record["version"] = json!(2),
                _ => unreachable!(),
            }
            write_private_replace(&marker, &serde_json::to_vec(&record).unwrap()).unwrap();
        } else {
            match defect {
                "record_corrupt" => fs::write(&marker, b"not json").unwrap(),
                "record_missing" => fs::remove_file(&marker).unwrap(),
                "record_oversize" => fs::write(&marker, vec![b' '; 8193]).unwrap(),
                "record_link" => {
                    fs::remove_file(&marker).unwrap();
                    symlink(&f.input, &marker).unwrap();
                }
                "unknown_file" => write_private_new(&dir.join("unknown"), b"retained").unwrap(),
                "unknown_dir" => private_dir(&dir.join("unknown")).unwrap(),
                "target_hash" => fs::write(&target, b"not selected audio").unwrap(),
                "partial_bytes" => {
                    fs::write(dir.join(".import-copy"), b"not a source prefix").unwrap()
                }
                "target_link" => {
                    fs::remove_file(&target).unwrap();
                    symlink(&f.input, &target).unwrap();
                }
                "external_hardlink" => {
                    fs::hard_link(&target, f._temp.path().join("external-link")).unwrap()
                }
                "unrelated_pair" => {
                    write_private_new(&dir.join(".import-copy"), &fs::read(&target).unwrap())
                        .unwrap()
                }
                "mode" => fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap(),
                "directory_mode" => {
                    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap()
                }
                "directory_link" => {
                    let actual = f._temp.path().join("relocated-promotion");
                    fs::rename(&dir, &actual).unwrap();
                    symlink(actual, &dir).unwrap();
                }
                "source_hash" => {
                    fs::write(f.stage(&h).join("imported-mix.wav"), b"changed source").unwrap()
                }
                "selection_missing" => fs::remove_file(f.stage(&h).join("selection.json")).unwrap(),
                _ => unreachable!(),
            }
        }
        let before = tree_snapshot(&f.paths.data);
        let cache = tree_snapshot(&f.paths.cache);
        let source = hash_file(&f.input).unwrap();
        // With no selection, only the original cache ownership is known. Keep
        // the existing failed-selection cache cleanup; never guess/scan DATA.
        if defect == "selection_missing" {
            imports::cancel(&f.paths, &f.owner, &h).unwrap();
            assert!(!f.stage(&h).exists());
            assert_eq!(tree_snapshot(&f.paths.data), before);
            assert_eq!(hash_file(&f.input).unwrap(), source);
            continue;
        }
        assert!(
            imports::cancel(&f.paths, &f.owner, &h).is_err(),
            "cancel accepted unsafe {defect}"
        );
        let report = imports::reap_expired(
            &f.paths,
            &f.owner,
            chrono::Utc::now().timestamp_millis() + 900001,
        )
        .unwrap();
        assert_eq!(report["removed"], 0, "unsafe {defect}");
        assert_eq!(report["retained"], 1, "unsafe {defect}");
        assert_eq!(
            tree_snapshot(&f.paths.data),
            before,
            "DATA changed for {defect}"
        );
        assert_eq!(
            tree_snapshot(&f.paths.cache),
            cache,
            "CACHE changed for {defect}"
        );
        assert_eq!(hash_file(&f.input).unwrap(), source);
    }
}
#[tokio::test]
async fn desktop_import_interrupted_abandon_committed_and_replayed_data_stay_readable() {
    for operation in ["cancel", "expiry", "replay"] {
        let f = Fixture::new();
        let h = f.select().await;
        let saved: Vec<_> = fs::read_dir(f.stage(&h))
            .unwrap()
            .map(|entry| {
                let p = entry.unwrap().path();
                (p.file_name().unwrap().to_owned(), fs::read(p).unwrap())
            })
            .collect();
        let request = f.request(&h);
        let result = imports::commit(&f.paths, &f.owner, &request).unwrap();
        let id = Uuid::parse_str(result["meeting"]["meetingId"].as_str().unwrap()).unwrap();
        let store = Store::open_read_only(&f.paths).unwrap();
        let meeting = store.meeting(id).unwrap().unwrap();
        let audio = &meeting.segments[0];
        let mut reader = media::VerifiedMedia::open(
            &meeting.directory,
            audio.path.strip_prefix(&meeting.directory).unwrap(),
            &audio.sha256,
            store.audio_read_lock().unwrap(),
            "audio/wav",
        )
        .unwrap();
        let initial = reader.response("GET", Some("bytes=0-43"), None).unwrap();
        assert_eq!(initial.status, 206);
        let expected = initial.body;
        private_dir(&f.stage(&h)).unwrap();
        for (name, bytes) in saved {
            write_private_new(&f.stage(&h).join(name), &bytes).unwrap();
        }
        let data = tree_snapshot(&f.paths.data);
        let db_hash = hash_file(&f.paths.state.join("state.sqlite")).unwrap();
        match operation {
            "cancel" => {
                imports::cancel(&f.paths, &f.owner, &h).unwrap();
            }
            "expiry" => {
                f.expire(&h);
                assert_eq!(
                    imports::reap_expired(
                        &f.paths,
                        &f.owner,
                        chrono::Utc::now().timestamp_millis()
                    )
                    .unwrap()["removed"],
                    1
                );
            }
            "replay" => assert_eq!(
                imports::commit(&f.paths, &f.owner, &request).unwrap(),
                result
            ),
            _ => unreachable!(),
        }
        assert!(!f.stage(&h).exists());
        assert_eq!(tree_snapshot(&f.paths.data), data, "committed DATA changed");
        let response = reader.response("GET", Some("bytes=0-43"), None).unwrap();
        assert_eq!(response.status, 206);
        assert_eq!(
            response.body, expected,
            "owned verified reader lost committed audio"
        );
        drop(reader); // FD and shared maintenance lock released before fixture teardown.
        eprintln!(
            "desktop_import_reader_receipt={}",
            json!({"operation":operation,"meetingId":id,"artifactSha256":audio.sha256,"rangeStatus":response.status,"readerReleased":true})
        );
        assert_eq!(
            hash_file(&f.paths.state.join("state.sqlite")).unwrap(),
            db_hash,
            "cleanup wrote DB"
        );
        assert_eq!(
            imports::commit(&f.paths, &f.owner, &request).unwrap(),
            result
        );
    }
}

#[tokio::test]
async fn desktop_import_interrupted_abandon_db_writer_commit_wins_before_cleanup_check() {
    let f = Fixture::new();
    let h = f.select().await;
    let dir = interrupted_promotion(&f, &h, "published");
    let selection = imports::load(&f.paths, &f.owner, &h).unwrap();
    let conn = rusqlite::Connection::open(f.paths.state.join("state.sqlite")).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    // Hold a real uncommitted DB writer, then let cancel acquire the import lock.
    conn.execute("INSERT INTO meetings VALUES(?1,?2)",rusqlite::params![selection.meeting_id.to_string(),serde_json::to_string(&json!({"session_id":selection.meeting_id,"title":"Synthetic pending DB commit","directory":dir,"segments":[],"sealed_at_ms":0})).unwrap()]).unwrap();
    let paths = f.paths.clone();
    let owner = f.owner.clone();
    let handle = h.clone();
    let (send, recv) = std::sync::mpsc::channel();
    let (entered, start) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        entered.send(()).unwrap();
        send.send(imports::cancel(&paths, &owner, &handle)).unwrap();
    });
    start
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    // Do not probe/steal the try-lock: doing so would itself make cancel fail.
    assert!(
        matches!(
            recv.recv_timeout(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ),
        "cancel escaped the DB writer fence"
    );
    assert!(f.stage(&h).join("imported-mix.wav").exists());
    let before = tree_snapshot(&f.paths.data);
    conn.execute_batch("COMMIT").unwrap();
    recv.recv_timeout(std::time::Duration::from_secs(8))
        .unwrap()
        .unwrap();
    thread.join().unwrap();
    assert!(!f.stage(&h).exists());
    assert_eq!(
        tree_snapshot(&f.paths.data),
        before,
        "concurrently committed audio was deleted"
    );
}

#[tokio::test]
async fn desktop_import_interrupted_abandon_preserves_foreign_meeting_path_and_replay_references() {
    for reference in ["meeting_path", "task_path", "segment_path", "replay"] {
        let f = Fixture::new();
        let h = f.select().await;
        let dir = interrupted_promotion(&f, &h, "published");
        let s = imports::load(&f.paths, &f.owner, &h).unwrap();
        let conn = rusqlite::Connection::open(f.paths.state.join("state.sqlite")).unwrap();
        match reference {
            "meeting_path" => {
                conn.execute(
                    "INSERT INTO meetings VALUES(?1,?2)",
                    rusqlite::params![
                        Uuid::new_v4().to_string(),
                        serde_json::to_string(&json!({"directory":dir})).unwrap()
                    ],
                )
                .unwrap();
            }
            "task_path" => {
                let foreign = Uuid::new_v4();
                conn.execute(
                    "INSERT INTO meetings VALUES(?1,'{}')",
                    [foreign.to_string()],
                )
                .unwrap();
                conn.execute("INSERT INTO tasks(id,session,attempt,input_hash,config_hash,document,stage,status,directory,config,segments) VALUES(?1,?2,1,'hash','hash','{}','transcribe','done',?3,'{}','[]')",rusqlite::params![Uuid::new_v4().to_string(),foreign.to_string(),dir.to_str().unwrap()]).unwrap();
            }
            "segment_path" => {
                let foreign = Uuid::new_v4().to_string();
                let task = Uuid::new_v4().to_string();
                conn.execute("INSERT INTO meetings VALUES(?1,'{}')", [&foreign])
                    .unwrap();
                conn.execute("INSERT INTO tasks(id,session,attempt,input_hash,config_hash,document,stage,status,directory,config,segments) VALUES(?1,?2,1,'hash','hash','{}','transcribe','done',?3,'{}','[]')",rusqlite::params![task,foreign,f.paths.data.to_str().unwrap()]).unwrap();
                conn.execute(
                    "INSERT INTO segmented_sessions VALUES(?1,?2,0)",
                    rusqlite::params![foreign, task],
                )
                .unwrap();
                conn.execute("INSERT INTO segment_jobs(session,sequence,offset_frames,frames,audio) VALUES(?1,0,0,1,?2)",rusqlite::params![foreign,serde_json::to_string(&json!({"path":dir.join("imported-mix.wav")})).unwrap()]).unwrap();
            }
            "replay" => {
                conn.execute(
                    "INSERT INTO desktop_requests VALUES('import_commit:synthetic','hash',?1)",
                    [
                        serde_json::to_string(&json!({"meeting":{"meetingId":s.meeting_id}}))
                            .unwrap(),
                    ],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let before = tree_snapshot(&f.paths.data);
        let db_hash = hash_file(&f.paths.state.join("state.sqlite")).unwrap();
        imports::cancel(&f.paths, &f.owner, &h).unwrap();
        assert!(!f.stage(&h).exists());
        assert_eq!(
            tree_snapshot(&f.paths.data),
            before,
            "reference lost: {reference}"
        );
        assert_eq!(
            hash_file(&f.paths.state.join("state.sqlite")).unwrap(),
            db_hash
        );
    }
}

#[tokio::test]
async fn desktop_import_interrupted_abandon_no_db_migration_or_gc_on_read_only_open() {
    let f = Fixture::new();
    let h = f.select().await;
    let dir = interrupted_promotion(&f, &h, "published");
    let conn = rusqlite::Connection::open(f.paths.state.join("state.sqlite")).unwrap();
    conn.execute_batch("DROP TABLE desktop_requests; DROP TABLE desktop_imports;")
        .unwrap();
    let data = tree_snapshot(&f.paths.data);
    let cache = tree_snapshot(&f.paths.cache);
    let db_hash = hash_file(&f.paths.state.join("state.sqlite")).unwrap();
    let read = Store::open_read_only(&f.paths).unwrap();
    assert!(read.meetings().unwrap().is_empty());
    drop(read);
    assert_eq!(tree_snapshot(&f.paths.data), data);
    assert_eq!(tree_snapshot(&f.paths.cache), cache);
    assert!(imports::cancel(&f.paths, &f.owner, &h).is_err());
    assert_eq!(tree_snapshot(&f.paths.data), data);
    assert_eq!(tree_snapshot(&f.paths.cache), cache);
    assert_eq!(
        hash_file(&f.paths.state.join("state.sqlite")).unwrap(),
        db_hash
    );
    assert!(dir.join("imported-mix.wav").exists());
    assert_eq!(conn.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('desktop_requests','desktop_imports')",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[tokio::test]
async fn desktop_import_interrupted_abandon_duplicate_commit_releases_only_its_orphan() {
    let f = Fixture::new();
    let first = f.select().await;
    let orphan = interrupted_promotion(&f, &first, "published");
    let second = f.select().await;
    let admitted = imports::commit(&f.paths, &f.owner, &f.request(&second)).unwrap();
    let id = Uuid::parse_str(admitted["meeting"]["meetingId"].as_str().unwrap()).unwrap();
    let meeting = Store::open_read_only(&f.paths)
        .unwrap()
        .meeting(id)
        .unwrap()
        .unwrap();
    let preserved = tree_snapshot(&meeting.directory);
    let request = f.request(&first);
    let duplicate = imports::commit(&f.paths, &f.owner, &request).unwrap();
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(
        duplicate["meeting"]["meetingId"],
        admitted["meeting"]["meetingId"]
    );
    assert!(!orphan.exists());
    assert!(!f.stage(&first).exists());
    assert_eq!(tree_snapshot(&meeting.directory), preserved);
    assert_eq!(
        imports::commit(&f.paths, &f.owner, &request).unwrap(),
        duplicate
    );
}

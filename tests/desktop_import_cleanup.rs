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
}

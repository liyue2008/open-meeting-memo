use omm::core::{contracts::*, paths::Paths, store::Store};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
use tempfile::TempDir;

fn roots(t: &TempDir) -> Paths {
    Paths::resolve(
        &t.path().canonicalize().unwrap().join("home"),
        &BTreeMap::new(),
    )
    .unwrap()
}
fn fixture(t: &TempDir) -> (Paths, Store, SealedMeeting, ConfigSnapshot) {
    let paths = roots(t);
    paths.create_private_dirs().unwrap();
    let session_id = uuid::Uuid::new_v4();
    let dir = paths.data.join("meetings").join(session_id.to_string());
    omm::core::paths::private_dir(&dir).unwrap();
    let path = dir.join("segment.wav");
    fs::write(&path, b"synthetic audio, not a personal meeting").unwrap();
    let m = SealedMeeting {
        session_id,
        title: "fixture".into(),
        directory: dir,
        segments: vec![AudioSegment {
            capture: None,
            path,
            duration_ms: 1000,
            sha256: omm::core::store::hash_bytes(b"synthetic audio, not a personal meeting"),
        }],
        sealed_at_ms: 10,
    };
    let store = Store::open(&paths).unwrap();
    let cfg = ConfigSnapshot {
        audio: Default::default(),
        asr: AsrConfig {
            model_version: "fixture".into(),
            ..AsrConfig::default()
        },
        pi: PiConfig {
            provider: "fixture".into(),
            model: "fixture".into(),
            prompt_version: "1".into(),
            ..PiConfig::default()
        },
        ..ConfigSnapshot::default()
    };
    (paths, store, m, cfg)
}
fn transcript() -> Transcript {
    Transcript {
        schema_version: 1,
        model_version: "fixture".into(),
        duration_ms: 1000,
        segments: vec![TranscriptSegment {
            source: AudioSource::Mic,
            start_ms: 0,
            end_ms: 900,
            text: "测试逐字稿".into(),
        }],
    }
}

#[test]
fn xdg_relative_values_are_ignored_and_persistent_roots_match_profile() {
    let t = TempDir::new().unwrap();
    let mut env = BTreeMap::new();
    env.insert("XDG_DATA_HOME".into(), "relative".into());
    let p = Paths::resolve(&t.path().join("home"), &env).unwrap();
    assert_eq!(p.data, t.path().join("home/.local/share/omm"));
    assert_eq!(p.warnings.len(), 1);
    fs::set_permissions(t.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let runtime =
        omm::core::runtime::RuntimeLocation::resolve(&env, || Ok(t.path().canonicalize().unwrap()))
            .unwrap();
    assert_eq!(runtime.source, "macos_replacement");
    assert!(!runtime.path.exists());
    let profile = omm::service::ServiceProfile::new(p.clone());
    assert!(profile.check_roots(&p).is_ok());
    let mut conflicting = p.clone();
    conflicting.state = t.path().join("other");
    assert_eq!(
        profile.check_roots(&conflicting).unwrap_err().code,
        ErrorCode::Conflict
    );
}

#[test]
fn runtime_requires_absolute_private_owned_directory_and_no_symlink() {
    let t = tempfile::Builder::new()
        .prefix("omm-runtime-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let p = roots(&t);
    let runtime = t.path().canonicalize().unwrap().join("runtime");
    fs::create_dir(&runtime).unwrap();
    let mut env = BTreeMap::new();
    env.insert("XDG_RUNTIME_DIR".into(), runtime.to_str().unwrap().into());
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(p.runtime(&env).unwrap_err().code, ErrorCode::Permission);
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(p.runtime(&env).unwrap(), runtime.join("omm"));
    std::os::unix::fs::symlink(runtime.join("nonexistent"), runtime.join("omm")).unwrap();
    assert_eq!(p.runtime(&env).unwrap_err().code, ErrorCode::Permission);
    fs::remove_file(runtime.join("omm")).unwrap();
    let link = t.path().join("link");
    std::os::unix::fs::symlink(&runtime, &link).unwrap();
    env.insert("XDG_RUNTIME_DIR".into(), link.to_str().unwrap().into());
    assert!(p.runtime(&env).is_err());
}

#[test]
fn queue_is_idempotent_expired_lease_cannot_commit_and_summary_is_automatic() {
    let t = TempDir::new().unwrap();
    let (p, mut s, m, cfg) = fixture(&t);
    let task = s.enqueue(&m, &cfg, false).unwrap();
    assert_eq!(task.id, s.enqueue(&m, &cfg, false).unwrap().id);
    let old = s.claim(100, 10).unwrap().unwrap();
    assert_eq!(old.stage, Stage::Transcribe);
    assert_eq!(
        s.commit_transcript(&old, &transcript(), 111)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    s.recover(111).unwrap();
    let current = s.claim(111, 100).unwrap().unwrap();
    assert_ne!(old.lease.token, current.lease.token);
    assert!(s.commit_transcript(&old, &transcript(), 112).is_err());
    s.commit_transcript(&current, &transcript(), 112).unwrap();
    let summary = s.claim(113, 100).unwrap().unwrap();
    assert_eq!(summary.stage, Stage::Summary);
    s.fail(
        &summary,
        &OmmError::new(ErrorCode::Backend, true, "fixture failure"),
        114,
    )
    .unwrap();
    let before = fs::read(s.artifact_path(task.id, Stage::Transcribe).unwrap()).unwrap();
    s.retry(task.id).unwrap();
    let retry = s.claim(115, 100).unwrap().unwrap();
    assert_eq!(retry.stage, Stage::Summary);
    s.commit_summary(
        &retry,
        &Summary {
            schema_version: 1,
            markdown: "纪要".into(),
            provider: "fixture".into(),
            model: "fixture".into(),
            prompt_version: "1".into(),
        },
        116,
    )
    .unwrap();
    assert_eq!(s.task(task.id).unwrap().status, TaskStatus::Completed);
    assert_eq!(
        before,
        fs::read(s.artifact_path(task.id, Stage::Transcribe).unwrap()).unwrap()
    );
    let forced = s.enqueue(&m, &cfg, true).unwrap();
    assert_eq!(forced.attempt, 2);
    assert_ne!(task.id, forced.id);
    drop(s);
    assert_eq!(
        Store::open(&p).unwrap().task(task.id).unwrap().status,
        TaskStatus::Completed
    );
}

#[test]
fn unknown_observation_does_not_end_a_meeting_and_weak_signals_do_not_start() {
    use omm::core::lifecycle::{Action, Lifecycle};
    let mut l = Lifecycle::new(15_000, 30_000);
    let observe = |state, evidence, at| MeetingObservation {
        state,
        evidence,
        observed_at_ms: at,
        source_id: Some("fixture".into()),
        title: None,
    };
    assert_eq!(
        l.observe(&observe(MeetingState::Active, Evidence::Weak, 0), true),
        Action::None
    );
    l.observe(&observe(MeetingState::Active, Evidence::Strong, 1), true);
    assert_eq!(
        l.observe(
            &observe(MeetingState::Active, Evidence::Strong, 15_001),
            true
        ),
        Action::Start
    );
    assert_eq!(
        l.observe(
            &observe(MeetingState::Unknown, Evidence::None, 99_000),
            true
        ),
        Action::None
    );
    assert_eq!(
        l.observe(
            &observe(MeetingState::Inactive, Evidence::Strong, 100_000),
            true
        ),
        Action::None
    );
    assert_eq!(
        l.observe(
            &observe(MeetingState::Inactive, Evidence::Strong, 130_000),
            true
        ),
        Action::Stop
    );
}

#[test]
fn configured_model_provenance_must_match_before_any_final_artifact_is_committed() {
    let t = TempDir::new().unwrap();
    let (_, mut store, meeting, cfg) = fixture(&t);
    let task = store.enqueue(&meeting, &cfg, false).unwrap();
    let claim = store.claim(100, 100).unwrap().unwrap();
    let mut wrong = transcript();
    wrong.model_version = "unexpected-model".into();
    assert_eq!(
        store
            .commit_transcript(&claim, &wrong, 101)
            .unwrap_err()
            .code,
        ErrorCode::Corrupt
    );
    assert_eq!(
        store.task(task.id).unwrap().status,
        TaskStatus::Transcribing
    );
    assert!(!store
        .artifact_path(task.id, Stage::Transcribe)
        .unwrap()
        .exists());
    store.commit_transcript(&claim, &transcript(), 102).unwrap();
    let claim = store.claim(103, 100).unwrap().unwrap();
    let mut summary = Summary {
        schema_version: 1,
        markdown: "fixture".into(),
        provider: "unexpected-provider".into(),
        model: "fixture".into(),
        prompt_version: "1".into(),
    };
    assert_eq!(
        store
            .commit_summary(&claim, &summary, 104)
            .unwrap_err()
            .code,
        ErrorCode::Corrupt
    );
    assert_eq!(store.task(task.id).unwrap().status, TaskStatus::Summarizing);
    assert!(!store
        .artifact_path(task.id, Stage::Summary)
        .unwrap()
        .exists());
    summary.provider = "fixture".into();
    store.commit_summary(&claim, &summary, 105).unwrap();
    assert_eq!(store.task(task.id).unwrap().status, TaskStatus::Completed);
}

//! Native desktop regressions: isolated profiles and synthetic bytes only; no desktop notifications.
use omm::{
    application::{
        self as app, desktop_contracts as dto,
        media::{self, VerifiedMedia},
        profile,
    },
    core::{
        config::AppConfig,
        config_transaction::ConfigTransaction,
        contracts::*,
        paths::*,
        runtime::RuntimeGuard,
        store::{hash_bytes, Store},
    },
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{symlink, PermissionsExt},
};
use tempfile::TempDir;
use uuid::Uuid;
struct Fixture {
    _temp: TempDir,
    paths: Paths,
    store: Store,
    task: Task,
    meeting: SealedMeeting,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
        let root = temp.path().canonicalize().unwrap();
        let paths = Paths::resolve(&root.join("home"), &BTreeMap::new()).unwrap();
        paths.create_private_dirs().unwrap();
        let mut cfg = AppConfig::default();
        cfg.notifications.enabled = false;
        cfg.pi.provider = "fixture".into();
        cfg.pi.model = "fixture".into();
        cfg.save(&paths).unwrap();
        let id = Uuid::new_v4();
        let dir = paths.data.join("meetings").join(id.to_string());
        private_dir(&dir).unwrap();
        let wav = media::silent_wav(1);
        let path = dir.join("segment.wav");
        write_private_new(&path, &wav).unwrap();
        let meeting = SealedMeeting {
            session_id: id,
            title: "synthetic-only".into(),
            directory: dir,
            sealed_at_ms: 1700000000000,
            segments: vec![AudioSegment {
                capture: None,
                path,
                duration_ms: 1000,
                sha256: hash_bytes(&wav),
            }],
        };
        let mut store = Store::open(&paths).unwrap();
        let task = store.enqueue(&meeting, &cfg.snapshot(), false).unwrap();
        Self {
            _temp: temp,
            paths,
            store,
            task,
            meeting,
        }
    }
    fn settled(&mut self) {
        let cancelled = self.store.desktop_cancel(self.task.id).unwrap();
        assert_eq!(cancelled["status"], "cancelled");
    }
    fn transcript(&mut self, text: &str) {
        let claim = self.store.claim_asr(1, 10000).unwrap().unwrap();
        self.store
            .commit_transcript(
                &claim,
                &Transcript {
                    schema_version: 1,
                    model_version: claim.context.config.asr.model_version.clone(),
                    duration_ms: claim.segments.iter().map(|s| s.duration_ms).sum(),
                    segments: vec![TranscriptSegment {
                        source: AudioSource::Mic,
                        start_ms: 0,
                        end_ms: 999,
                        text: text.into(),
                    }],
                },
                2,
            )
            .unwrap();
    }
}
fn meta(title: &str) -> dto::MeetingMetadata {
    serde_json::from_value(json!({"title":title,"note":"fixture note","tags":["synthetic"]}))
        .unwrap()
}
fn request(payload: Value) -> Value {
    json!({"schemaVersion":1,"requestId":Uuid::new_v4().to_string(),"payload":payload})
}
#[test]
fn desktop_metadata_cas_idempotency_rollback_and_retained_trash() {
    let mut f = Fixture::new();
    f.settled();
    let id = f.meeting.session_id;
    let req = request(
        json!({"meetingId":id,"expectedRevision":0,"metadata":meta("renamed"),"admission":{"token":"one"}}),
    );
    let first = f
        .store
        .desktop_atomic("meeting_update", &req, |s| {
            app::mutate(s, &f.paths, "meeting_update", &req["payload"])
        })
        .unwrap();
    assert_eq!(first["revision"], 1);
    let mut repeat = req.clone();
    repeat["payload"]["admission"]["token"] = json!("rotated");
    assert_eq!(
        first,
        f.store
            .desktop_atomic("meeting_update", &repeat, |_| panic!("must deduplicate"))
            .unwrap()
    );
    let mut different = req.clone();
    different["payload"]["metadata"]["title"] = json!("other");
    assert_eq!(
        f.store
            .desktop_atomic("meeting_update", &different, |_| panic!())
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let fail = request(json!({}));
    let result = f.store.desktop_atomic("rollback", &fail, |s| {
        s.desktop_update(id, 1, &meta("rollback"))?;
        Err(OmmError::invalid("synthetic failure"))
    });
    assert!(result.is_err());
    assert_eq!(
        f.store.desktop_row(id).unwrap()["metadata"]["title"],
        "renamed"
    );
    let rev = |expected| {
        serde_json::from_value::<dto::MeetingRevision>(
            json!({"meetingId":id,"expectedRevision":expected}),
        )
        .unwrap()
    };
    let hash = omm::core::store::hash_file(&f.meeting.segments[0].path).unwrap();
    f.store.desktop_trash(&[rev(1)], true).unwrap();
    assert!(f.store.desktop_trash(&[rev(1)], false).is_err());
    f.store.desktop_trash(&[rev(2)], false).unwrap();
    assert_eq!(f.store.desktop_row(id).unwrap()["revision"], 3);
    assert_eq!(
        omm::core::store::hash_file(&f.meeting.segments[0].path).unwrap(),
        hash
    );
    drop(f.store);
    let store = Store::open_read_only(&f.paths).unwrap();
    assert_eq!(
        store.desktop_row(id).unwrap()["metadata"]["title"],
        "renamed"
    );
}
#[test]
fn desktop_cas_concurrent_writers_have_single_winner() {
    let mut f = Fixture::new();
    f.settled();
    let id = f.meeting.session_id;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writers: Vec<_> = ["one", "two"]
        .into_iter()
        .map(|title| {
            let paths = f.paths.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = Store::open(&paths).unwrap();
                barrier.wait();
                let req =
                    request(json!({"meetingId":id,"expectedRevision":0,"metadata":meta(title)}));
                store.desktop_atomic("meeting_update", &req, |store| {
                    store.desktop_update(id, 0, &meta(title))
                })
            })
        })
        .collect();
    let results: Vec<_> = writers
        .into_iter()
        .map(|writer| writer.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .next()
            .unwrap()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(f.store.desktop_row(id).unwrap()["revision"], 1);
}
#[test]
fn desktop_active_work_rejects_edit_trash_and_cancel_is_durable() {
    let mut f = Fixture::new();
    let row = f.store.desktop_row(f.meeting.session_id).unwrap();
    assert_eq!(row["capabilities"]["reason"], "busy");
    app::validation::validate_named("MeetingRow", &row).unwrap();
    assert!(f
        .store
        .desktop_update(f.meeting.session_id, 0, &meta("bad"))
        .is_err());
    let claim = f.store.claim_asr(1, 10000).unwrap().unwrap();
    let row = f.store.desktop_cancel(f.task.id).unwrap();
    assert_eq!(row["cancelRequested"], true);
    assert_eq!(row["canRequeue"], false);
    assert!(f.store.claim_asr(2, 10000).unwrap().is_none());
    drop(claim);
    f.store.settle_cancellation(f.task.id).unwrap();
    drop(f.store);
    let store = Store::open_read_only(&f.paths).unwrap();
    assert_eq!(store.desktop_task(f.task.id).unwrap()["canRequeue"], true);
}
#[test]
fn desktop_content_search_pagination_and_artifact_versions() {
    let mut f = Fixture::new();
    f.transcript("unique_transcript_search_token");
    let mut query: Value = serde_json::from_slice(include_bytes!(
        "../contracts/desktop/v1/examples/meeting_list.json"
    ))
    .unwrap();
    query = query["request"]["payload"].clone();
    query["filter"]["query"] = json!("unique_transcript_search_token");
    query["filter"]["scope"] = json!("content");
    let filter: dto::MeetingListRequest = serde_json::from_value(query.clone()).unwrap();
    let list = f.store.desktop_list(&filter).unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    let bytes = f.store.read_artifact(f.task.id, Stage::Transcribe).unwrap();
    let old = hash_bytes(&bytes);
    let claim = f.store.claim_summary(3, 10000).unwrap().unwrap();
    f.store
        .commit_summary(
            &claim,
            &Summary {
                schema_version: 1,
                markdown: "# unique_summary_search_token".into(),
                provider: claim.context.config.pi.provider.clone(),
                model: claim.context.config.pi.model.clone(),
                prompt_version: claim.context.config.pi.prompt_version.clone(),
            },
            4,
        )
        .unwrap();
    query["filter"]["query"] = json!("unique_summary_search_token");
    assert_eq!(
        f.store
            .desktop_list(&serde_json::from_value(query).unwrap())
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let versions = f.store.desktop_versions(f.meeting.session_id).unwrap();
    assert!(versions
        .iter()
        .any(|v| v["current"] == true && v["artifact"]["stage"] == "summary"));
    let cfg = AppConfig::load(&f.paths).unwrap();
    let next = f.store.enqueue(&f.meeting, &cfg.snapshot(), true).unwrap();
    assert_ne!(next.id, f.task.id);
    assert_eq!(
        f.store.desktop_row(f.meeting.session_id).unwrap()["currentTranscriptTaskId"],
        f.task.id.to_string()
    );
    assert_eq!(
        hash_bytes(&f.store.read_artifact(f.task.id, Stage::Transcribe).unwrap()),
        old
    );
}
#[test]
fn desktop_config_raw_hash_and_writer_lock_prevent_lost_updates() {
    let f = Fixture::new();
    let first = AppConfig::load(&f.paths).unwrap();
    let mut second = AppConfig::load(&f.paths).unwrap();
    second.poll_ms = 2500;
    second.save(&f.paths).unwrap();
    assert_eq!(first.save(&f.paths).unwrap_err().code, ErrorCode::Conflict);
    let _lock = ConfigTransaction::acquire(&f.paths).unwrap();
    assert_eq!(
        AppConfig::load(&f.paths)
            .unwrap()
            .save(&f.paths)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}
#[test]
fn desktop_media_verified_fd_ranges_hash_and_symlink_safety() {
    let f = Fixture::new();
    let relative = f.meeting.segments[0]
        .path
        .strip_prefix(&f.paths.data)
        .unwrap();
    let mut media = VerifiedMedia::open(
        &f.paths.data,
        relative,
        &f.meeting.segments[0].sha256,
        f.store.audio_read_lock().unwrap(),
        "audio/wav",
    )
    .unwrap();
    assert_eq!(
        media
            .response("GET", Some("bytes=0-43"), None)
            .unwrap()
            .status,
        206
    );
    assert_eq!(media.response("HEAD", None, None).unwrap().body.len(), 0);
    for range in ["bytes=0-1,4-5", "bytes=999999-", "bytes=-0", "bytes=2-1"] {
        assert_eq!(
            media.response("GET", Some(range), None).unwrap().status,
            416
        );
    }
    assert_eq!(media.response("POST", None, None).unwrap().status, 405);
    assert_eq!(
        media
            .response("GET", Some("bytes=0-1"), Some("wrong"))
            .unwrap()
            .status,
        412
    );
    assert!(f.store.active_audio_readers().unwrap());
    assert!(VerifiedMedia::open(
        &f.paths.data,
        relative,
        &"0".repeat(64),
        f.store.audio_read_lock().unwrap(),
        "audio/wav"
    )
    .is_err());
    let path = f.paths.data.join("link");
    symlink(&f.meeting.directory, &path).unwrap();
    assert!(VerifiedMedia::open(
        &f.paths.data,
        std::path::Path::new("link/segment.wav"),
        &f.meeting.segments[0].sha256,
        f.store.audio_read_lock().unwrap(),
        "audio/wav"
    )
    .is_err());
    fs::rename(
        &f.meeting.segments[0].path,
        f.meeting.directory.join("renamed.wav"),
    )
    .unwrap();
    assert_eq!(
        media.response("GET", Some("bytes=0-3"), None).unwrap().body,
        b"RIFF"
    );
    use std::io::{Seek, SeekFrom, Write};
    let mut external = fs::OpenOptions::new()
        .write(true)
        .open(f.meeting.directory.join("renamed.wav"))
        .unwrap();
    external.seek(SeekFrom::Start(0)).unwrap();
    external.write_all(b"EDIT").unwrap();
    external.sync_all().unwrap();
    assert_eq!(
        media
            .response("GET", Some("bytes=0-3"), None)
            .err()
            .unwrap()
            .code,
        ErrorCode::Corrupt
    );
    drop(media);
}
#[test]
fn desktop_legacy_profile_read_does_not_migrate_or_grant_writes() {
    let f = Fixture::new();
    let path = f.paths.state.join("state.sqlite");
    let before = fs::read(&path).unwrap();
    assert!(profile::read(&f.paths).unwrap().is_none());
    let store = Store::open_read_only(&f.paths).unwrap();
    store.desktop_row(f.meeting.session_id).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    let runtime = f._temp.path().join("runtime");
    private_dir(&runtime).unwrap();
    let location = f
        .paths
        .runtime_location(&BTreeMap::from([(
            "XDG_RUNTIME_DIR".into(),
            runtime.to_string_lossy().into_owned(),
        )]))
        .unwrap();
    let guard = RuntimeGuard::acquire(&location, &f.paths.state).unwrap();
    let owner = profile::takeover(&f.paths, &guard).unwrap();
    let admission = json!({"ownerGeneration":owner.generation,"profileId":owner.profile_id,"stateSchemaVersion":1,"expiresAtMs":chrono::Utc::now().timestamp_millis()+1000});
    assert!(profile::verify_admission(&owner, &admission).is_ok());
    let mut changed = admission;
    changed["ownerGeneration"] = json!(Uuid::new_v4());
    assert!(profile::verify_admission(&owner, &changed).is_err());
    assert!(profile::offline_lock(&f.paths).is_err());
}
#[test]
fn desktop_media_permission_change_is_rejected() {
    let f = Fixture::new();
    fs::set_permissions(
        &f.meeting.segments[0].path,
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(VerifiedMedia::open(
        &f.paths.data,
        f.meeting.segments[0]
            .path
            .strip_prefix(&f.paths.data)
            .unwrap(),
        &f.meeting.segments[0].sha256,
        f.store.audio_read_lock().unwrap(),
        "audio/wav"
    )
    .is_err());
}

fn owner(paths: &Paths) -> profile::ProfileOwner {
    profile::ProfileOwner {
        schema_version: 1,
        min_reader_version: 1,
        generation: Uuid::new_v4(),
        profile_id: profile::profile_id(paths).unwrap(),
    }
}
#[tokio::test]
async fn desktop_import_normalizes_mixed_lineage_deduplicates_and_binds_generation() {
    let f = Fixture::new();
    let owner = owner(&f.paths);
    let selected = app::imports::select(&f.paths, &owner, &f.meeting.segments[0].path)
        .await
        .unwrap();
    let handle = selected["selection"]["selectionHandle"].as_str().unwrap();
    let req = request(
        json!({"selectionHandle":handle,"metadata":meta("imported"),"startedAtMs":1700000000000i64,"timeZone":"Asia/Shanghai"}),
    );
    let result = app::imports::commit(&f.paths, &owner, &req).unwrap();
    assert_eq!(result["duplicate"], false);
    app::validation::validate_named("ImportResult", &result).unwrap();
    let meeting = Uuid::parse_str(result["meeting"]["meetingId"].as_str().unwrap()).unwrap();
    let store = Store::open_read_only(&f.paths).unwrap();
    assert!(store.desktop_imported(meeting).unwrap());
    assert_eq!(store.desktop_row(meeting).unwrap()["source"], "imported");
    let versions = store.tasks().unwrap();
    let imported = versions
        .iter()
        .find(|task| task.session_id == meeting)
        .unwrap();
    let imported_meeting = store.meeting(imported.session_id).unwrap().unwrap();
    let segment = &imported_meeting.segments[0];
    assert!(segment.path.ends_with("imported-mix.wav"));
    assert_eq!(segment.sha256, f.meeting.segments[0].sha256);
    assert_eq!(
        app::imports::commit(&f.paths, &owner, &req).unwrap(),
        result
    );
    let another = app::imports::select(&f.paths, &owner, &f.meeting.segments[0].path)
        .await
        .unwrap();
    let mut duplicate = req.clone();
    duplicate["requestId"] = json!(Uuid::new_v4());
    duplicate["payload"]["selectionHandle"] = another["selection"]["selectionHandle"].clone();
    assert_eq!(
        app::imports::commit(&f.paths, &owner, &duplicate).unwrap()["duplicate"],
        true
    );
    let changed = profile::ProfileOwner {
        generation: Uuid::new_v4(),
        ..owner.clone()
    };
    assert!(app::imports::load(&f.paths, &changed, handle).is_err());
    app::imports::cancel(&f.paths, &owner, handle).unwrap();
    assert!(app::imports::load(&f.paths, &owner, handle).is_err());
    assert!(app::imports::load(&f.paths, &owner, "../../anything").is_err());
}
#[test]
fn desktop_configuration_journal_recovers_published_target_without_reapplying() {
    let f = Fixture::new();
    let owner = owner(&f.paths);
    let old = ConfigTransaction::acquire(&f.paths)
        .unwrap()
        .revision
        .clone();
    let req = request(
        json!({"expectedRevision":old,"patch":{"pollMs":2750},"admission":{"token":"test"}}),
    );
    let first = app::configuration_journal::save(&f.paths, &owner, "config_save", &req).unwrap();
    let path = f.paths.config.join("desktop-journals").join(format!(
        "{}.json",
        hash_bytes(req["requestId"].as_str().unwrap().as_bytes())
    ));
    let mut journal: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    journal["result"] = Value::Null;
    write_private_replace(&path, &serde_json::to_vec(&journal).unwrap()).unwrap();
    let bytes = fs::read(f.paths.config.join("config.json")).unwrap();
    let recovered =
        app::configuration_journal::save(&f.paths, &owner, "config_save", &req).unwrap();
    assert_eq!(first, recovered);
    assert_eq!(bytes, fs::read(f.paths.config.join("config.json")).unwrap());
    let mut wrong = req.clone();
    wrong["payload"]["patch"]["pollMs"] = json!(3000);
    assert_eq!(
        app::configuration_journal::save(&f.paths, &owner, "config_save", &wrong)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}
#[test]
fn desktop_unknown_future_owner_is_not_downgraded_on_explicit_service_start() {
    let f = Fixture::new();
    let path = f.paths.state.join("desktop-owner.json");
    let mut future = serde_json::to_value(owner(&f.paths)).unwrap();
    future["minReaderVersion"] = json!(2);
    let bytes = serde_json::to_vec(&future).unwrap();
    write_private_new(&path, &bytes).unwrap();
    let runtime = f._temp.path().join("runtime-future");
    private_dir(&runtime).unwrap();
    let location = f
        .paths
        .runtime_location(&BTreeMap::from([(
            "XDG_RUNTIME_DIR".into(),
            runtime.to_string_lossy().into_owned(),
        )]))
        .unwrap();
    let guard = RuntimeGuard::acquire(&location, &f.paths.state).unwrap();
    assert_eq!(
        profile::takeover(&f.paths, &guard).err().unwrap().code,
        ErrorCode::Conflict
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
}
#[test]
fn desktop_frozen_request_validation_rejects_paths_unknown_fields_and_byte_overflow() {
    let mut req: Value = serde_json::from_slice(include_bytes!(
        "../contracts/desktop/v1/examples/media_open.json"
    ))
    .unwrap();
    req = req["request"].clone();
    req["payload"]["path"] = json!("/etc/passwd");
    assert!(app::validation::request("media_open", &req).is_err());
    let long = meta(&"合".repeat(100));
    let update = request(
        json!({"meetingId":Uuid::new_v4(),"expectedRevision":0,"metadata":long,"admission":{"token":"test"}}),
    );
    assert!(app::validation::request("meeting_update", &update).is_err());
    assert_eq!(
        serde_json::to_value(profile::capability_request()).unwrap()["intent"],
        "mutation"
    );
}

struct NoSummary;
#[async_trait::async_trait]
impl SummaryAdapter for NoSummary {
    async fn summarize(&self, _: SummaryRequest) -> Result<Summary> {
        panic!("cancellation fixture must not invoke summary")
    }
}
#[tokio::test]
async fn desktop_persistent_cancel_waits_for_owned_worker_before_settling() {
    let mut f = Fixture::new();
    let worker = f._temp.path().join("owned-fixture-worker");
    let pidfile = f._temp.path().join("owned-worker-pid");
    write_private_new(
        &worker,
        format!(
            "#!/bin/sh\nprintf '%s' $$ > '{}'\nexec /bin/sleep 30\n",
            pidfile.display()
        )
        .as_bytes(),
    )
    .unwrap();
    fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
    let asr = omm::adapters::asr::LocalAsrAdapter::with_paths(worker, f.paths.clone()).unwrap();
    let mut control = Store::open(&f.paths).unwrap();
    let task = f.task.id;
    let cancel = async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !pidfile.exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let pid: i32 = fs::read_to_string(&pidfile).unwrap().parse().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        let state = control.desktop_cancel(task).unwrap();
        assert_eq!(state["cancelRequested"], true);
        assert_eq!(state["canRequeue"], false);
        pid
    };
    let (result, pid) = tokio::join!(
        omm::core::pipeline::run_one(&mut f.store, &asr, &NoSummary, 5000),
        cancel
    );
    assert_eq!(result.unwrap().unwrap().status, TaskStatus::Blocked);
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    let state = f.store.desktop_task(task).unwrap();
    assert_eq!(state["status"], "cancelled");
    assert_eq!(state["canRequeue"], true);
    assert!(f.meeting.segments[0].path.exists());
    assert!(!f.store.active_audio_readers().unwrap());
}
#[tokio::test]
async fn desktop_cli_reads_retained_success_and_shares_metadata_and_trash() {
    use clap::Parser;
    use omm::cli::{execute, Cli};
    let mut f = Fixture::new();
    f.transcript("retained successful transcript");
    f.store.desktop_cancel(f.task.id).unwrap();
    let config = AppConfig::load(&f.paths).unwrap();
    let next = f
        .store
        .enqueue(&f.meeting, &config.snapshot(), true)
        .unwrap();
    f.store.desktop_cancel(next.id).unwrap();
    f.store
        .desktop_update(
            f.meeting.session_id,
            0,
            &meta("renamed through shared use case"),
        )
        .unwrap();
    let id = f.meeting.session_id.to_string();
    let read =
        Cli::try_parse_from(["omm", "meeting", "read", &id, "--artifact", "transcript"]).unwrap();
    let result = execute(&read.command, &f.paths, true).await.unwrap();
    assert!(result["content"]
        .as_str()
        .unwrap()
        .contains("retained successful transcript"));
    let list = Cli::try_parse_from(["omm", "meeting", "list"]).unwrap();
    assert_eq!(
        execute(&list.command, &f.paths, true).await.unwrap()[0]["title"],
        "renamed through shared use case"
    );
    f.store
        .desktop_trash(
            &[dto::MeetingRevision {
                meeting_id: id.clone(),
                expected_revision: 1,
            }],
            true,
        )
        .unwrap();
    assert!(execute(&list.command, &f.paths, true)
        .await
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
    let trash = Cli::try_parse_from(["omm", "meeting", "list", "--trash"]).unwrap();
    assert_eq!(
        execute(&trash.command, &f.paths, true)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for args in [
        vec![
            "omm",
            "meeting",
            "update",
            &id,
            "--expected-revision",
            "2",
            "--title",
            "fixture",
        ],
        vec!["omm", "meeting", "restore", &id, "--expected-revision", "2"],
        vec!["omm", "task", "cancel", &next.id.to_string()],
        vec!["omm", "task", "requeue", &next.id.to_string()],
    ] {
        assert!(Cli::try_parse_from(args).is_ok());
    }
}

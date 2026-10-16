use omm::core::{
    contracts::*,
    paths::{private_dir, write_private_new, Paths},
    store::{hash_bytes, Store},
};
use std::{collections::BTreeMap, fs};
use tempfile::TempDir;

#[test]
fn prepared_commit_survives_restart_before_or_after_atomic_rename() {
    for renamed in [false, true] {
        let t = TempDir::new().unwrap();
        let root = t.path().canonicalize().unwrap();
        let paths = Paths::resolve(&root.join("home"), &BTreeMap::new()).unwrap();
        paths.create_private_dirs().unwrap();
        let session_id = uuid::Uuid::new_v4();
        let dir = paths.data.join("meetings").join(session_id.to_string());
        private_dir(&dir).unwrap();
        let path = dir.join("audio.wav");
        fs::write(&path, b"synthetic").unwrap();
        let meeting = SealedMeeting {
            session_id,
            title: "fixture".into(),
            directory: dir,
            sealed_at_ms: 1,
            segments: vec![AudioSegment {
                capture: None,
                path,
                duration_ms: 1000,
                sha256: hash_bytes(b"synthetic"),
            }],
        };
        let mut store = Store::open(&paths).unwrap();
        let task = store
            .enqueue(&meeting, &ConfigSnapshot::default(), false)
            .unwrap();
        let claim = store.claim(10, 100).unwrap().unwrap();
        let dest = store.artifact_path(task.id, Stage::Transcribe).unwrap();
        private_dir(dest.parent().unwrap()).unwrap();
        let tmp = dest.parent().unwrap().join("pending.tmp");
        let transcript = Transcript {
            schema_version: 1,
            model_version: "fixture".into(),
            duration_ms: 1000,
            segments: vec![],
        };
        let bytes = serde_json::to_vec(&transcript).unwrap();
        write_private_new(&tmp, &bytes).unwrap();
        // Seed the real durable seam at the two crash windows, not an alternate file-only queue.
        let db = rusqlite::Connection::open(paths.state.join("state.sqlite")).unwrap();
        db.execute("INSERT INTO commits(task,stage,lease,staging,destination,hash) VALUES(?1,'transcribe',?2,?3,?4,?5)", rusqlite::params![task.id.to_string(),claim.lease.token.to_string(),tmp.to_str(),dest.to_str(),hash_bytes(&bytes)]).unwrap();
        drop(db);
        if renamed {
            fs::rename(&tmp, &dest).unwrap();
        }
        drop(store);
        let mut recovered = Store::open(&paths).unwrap();
        recovered.recover(1000).unwrap();
        assert_eq!(
            recovered.task(task.id).unwrap().status,
            TaskStatus::TranscriptCommitted
        );
        assert_eq!(
            recovered.claim(1001, 100).unwrap().unwrap().stage,
            Stage::Summary
        );
        assert_eq!(
            recovered.read_artifact(task.id, Stage::Transcribe).unwrap(),
            bytes
        );
        recovered.recover(1002).unwrap();
        assert_eq!(fs::read_dir(dest.parent().unwrap()).unwrap().count(), 1);
    }
}

#[test]
fn malformed_timestamps_and_audio_hashes_are_rejected() {
    let t = TempDir::new().unwrap();
    let root = t.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &BTreeMap::new()).unwrap();
    paths.create_private_dirs().unwrap();
    let session_id = uuid::Uuid::new_v4();
    let dir = paths.data.join("meetings").join(session_id.to_string());
    private_dir(&dir).unwrap();
    let path = dir.join("audio.wav");
    fs::write(&path, b"synthetic").unwrap();
    let mut meeting = SealedMeeting {
        session_id,
        title: "fixture".into(),
        directory: dir,
        sealed_at_ms: 1,
        segments: vec![AudioSegment {
            capture: None,
            path,
            duration_ms: 1000,
            sha256: "bad".into(),
        }],
    };
    let mut store = Store::open(&paths).unwrap();
    assert_eq!(
        store
            .enqueue(&meeting, &ConfigSnapshot::default(), false)
            .unwrap_err()
            .code,
        ErrorCode::Corrupt
    );
    meeting.segments[0].sha256 = hash_bytes(b"synthetic");
    let task = store
        .enqueue(&meeting, &ConfigSnapshot::default(), false)
        .unwrap();
    let claim = store.claim(10, 100).unwrap().unwrap();
    let transcript = Transcript {
        schema_version: 1,
        model_version: "fixture".into(),
        duration_ms: 1000,
        segments: vec![TranscriptSegment {
            source: AudioSource::System,
            start_ms: 900,
            end_ms: 1100,
            text: "fixture".into(),
        }],
    };
    assert_eq!(
        store
            .commit_transcript(&claim, &transcript, 11)
            .unwrap_err()
            .code,
        ErrorCode::Corrupt
    );
    assert!(store.read_artifact(task.id, Stage::Transcribe).is_err());
}

#[test]
fn imported_summary_attempt_retains_asr_provenance_and_recovers_each_commit_window() {
    for renamed in [false, true] {
        let t = TempDir::new().unwrap();
        let paths = Paths::resolve(
            &t.path().canonicalize().unwrap().join("home"),
            &BTreeMap::new(),
        )
        .unwrap();
        paths.create_private_dirs().unwrap();
        let id = uuid::Uuid::new_v4();
        let directory = paths.data.join("meetings").join(id.to_string());
        private_dir(&directory).unwrap();
        let audio = directory.join("audio.wav");
        fs::write(&audio, b"synthetic").unwrap();
        let meeting = SealedMeeting {
            session_id: id,
            title: "synthetic".into(),
            directory,
            sealed_at_ms: 1,
            segments: vec![AudioSegment {
                capture: None,
                path: audio,
                duration_ms: 1000,
                sha256: hash_bytes(b"synthetic"),
            }],
        };
        let mut cfg = ConfigSnapshot::default();
        cfg.asr.model_version = "original-asr".into();
        cfg.pi.provider = "original-provider".into();
        cfg.pi.model = "original-model".into();
        cfg.pi.prompt_version = "original-prompt".into();
        let mut store = Store::open(&paths).unwrap();
        let source = store.enqueue(&meeting, &cfg, false).unwrap();
        let claim = store.claim(10, 100).unwrap().unwrap();
        let transcript = Transcript {
            schema_version: 1,
            model_version: "original-asr".into(),
            duration_ms: 1000,
            segments: vec![],
        };
        store.commit_transcript(&claim, &transcript, 11).unwrap();
        let claim = store.claim(12, 100).unwrap().unwrap();
        store
            .commit_summary(
                &claim,
                &Summary {
                    schema_version: 1,
                    markdown: "original minutes".into(),
                    provider: cfg.pi.provider.clone(),
                    model: cfg.pi.model.clone(),
                    prompt_version: cfg.pi.prompt_version.clone(),
                },
                13,
            )
            .unwrap();
        let mut pi = cfg.pi.clone();
        pi.model = "new-summary-model".into();
        let mut imported = store.force_summary(source.id, "new-version", &pi).unwrap();
        let provenance = imported.transcript_source.clone().unwrap();
        assert_eq!(provenance.task_id, source.id);
        assert_eq!(provenance.config_hash, source.config_hash);
        assert_eq!(provenance.model_version, "original-asr");
        let db = rusqlite::Connection::open(paths.state.join("state.sqlite")).unwrap();
        let (token,staging,dest):(String,String,String)=db.query_row("SELECT lease,staging,destination FROM commits WHERE task=?1 AND stage='transcribe'",[imported.id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        imported.stage = Stage::Transcribe;
        imported.status = TaskStatus::Transcribing;
        db.execute("UPDATE tasks SET document=?2,stage='transcribe',status='transcribing',lease=?3,expires=0 WHERE id=?1",rusqlite::params![imported.id.to_string(),serde_json::to_string(&imported).unwrap(),token]).unwrap();
        db.execute(
            "UPDATE commits SET done=0 WHERE task=?1",
            [imported.id.to_string()],
        )
        .unwrap();
        if !renamed {
            fs::rename(&dest, &staging).unwrap();
        }
        drop(db);
        drop(store);
        let mut recovered = Store::open(&paths).unwrap();
        recovered.recover(1000).unwrap();
        let task = recovered.task(imported.id).unwrap();
        assert_eq!(task.stage, Stage::Summary);
        assert_eq!(task.status, TaskStatus::TranscriptCommitted);
        assert_eq!(task.transcript_source.unwrap().sha256, provenance.sha256);
        let claim = recovered.claim(1001, 100).unwrap().unwrap();
        assert_eq!(claim.task.id, imported.id);
        assert_eq!(claim.stage, Stage::Summary);
        assert_eq!(claim.context.config.asr.model_version, "original-asr");
        assert_eq!(claim.context.config.pi.model, "new-summary-model");
        assert!(recovered.read_artifact(source.id, Stage::Summary).is_ok());
    }
}

#[test]
fn retry_cleanup_preserves_chunks_but_reclaims_only_dead_attempts_across_restart() {
    let t = TempDir::new().unwrap();
    let root = t.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &BTreeMap::new()).unwrap();
    paths.create_private_dirs().unwrap();
    let session_id = uuid::Uuid::new_v4();
    let directory = paths.data.join("meetings").join(session_id.to_string());
    private_dir(&directory).unwrap();
    let audio = directory.join("audio.wav");
    write_private_new(&audio, b"generated").unwrap();
    let meeting = SealedMeeting {
        session_id,
        title: "generated".into(),
        directory,
        sealed_at_ms: 1,
        segments: vec![AudioSegment {
            capture: None,
            path: audio,
            sha256: hash_bytes(b"generated"),
            duration_ms: 1000,
        }],
    };
    let mut store = Store::open(&paths).unwrap();
    let task = store
        .enqueue(&meeting, &ConfigSnapshot::default(), false)
        .unwrap();
    let claim = store.claim(10, 100).unwrap().unwrap();
    let owner = claim.context.work_dir.parent().unwrap();
    let chunks = owner.join(".chunks");
    private_dir(&chunks).unwrap();
    write_private_new(&chunks.join("committed"), b"generated checkpoint").unwrap();
    write_private_new(&claim.context.work_dir.join("temporary"), b"disposable").unwrap();
    store
        .fail(
            &claim,
            &OmmError::new(ErrorCode::Timeout, true, "generated timeout"),
            11,
        )
        .unwrap();
    assert!(store
        .schedule_retry(&store.task(task.id).unwrap(), 12)
        .unwrap());
    store.clean_abandoned_cache(200).unwrap();
    assert!(
        chunks.join("committed").exists(),
        "retry lost durable chunks"
    );
    assert!(
        !claim.context.work_dir.exists(),
        "dead lease workspace retained"
    );
    drop(store);
    let mut store = Store::open(&paths).unwrap();
    store.recover(2000).unwrap();
    store.clean_abandoned_cache(2000).unwrap();
    assert!(chunks.join("committed").exists());
    let retry = store.claim(2000, 100).unwrap().unwrap();
    assert_eq!(retry.task.id, task.id);
    assert_eq!(retry.context.work_dir.parent(), Some(owner));
}

#[test]
fn legacy_progress_and_first_error_wall_time_persist_without_private_message() {
    use omm::core::store::{StageAttemptError, StageErrorPhase};
    let t = TempDir::new().unwrap();
    let root = t.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &BTreeMap::new()).unwrap();
    paths.create_private_dirs().unwrap();
    let session_id = uuid::Uuid::new_v4();
    let dir = paths.data.join("meetings").join(session_id.to_string());
    private_dir(&dir).unwrap();
    let path = dir.join("audio.wav");
    write_private_new(&path, b"synthetic").unwrap();
    let meeting = SealedMeeting {
        session_id,
        title: "fixture".into(),
        directory: dir,
        sealed_at_ms: 1,
        segments: vec![AudioSegment {
            capture: None,
            path,
            duration_ms: 1000,
            sha256: hash_bytes(b"synthetic"),
        }],
    };
    let mut store = Store::open(&paths).unwrap();
    let task = store
        .enqueue(&meeting, &ConfigSnapshot::default(), false)
        .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let claim = store.claim(now, 60000).unwrap().unwrap();
    let event = AsrWorkerEvent::ChunkProgress {
        schema_version: 1,
        task_id: task.id,
        attempt: claim.task.attempt,
        source: AudioSource::System,
        completed_chunks: 2,
        vad_spans: 3,
        speech_samples: 16000,
        processing_ms: 1234,
        startup_timing: None,
        chunk_timing: None,
    };
    store
        .record_segment_progress(&claim.context, &event)
        .unwrap();
    let progress = store
        .state::<serde_json::Value>(&format!("stage-progress:{}", task.id))
        .unwrap()
        .unwrap();
    assert_eq!(progress["event"]["completed_chunks"], 2);
    assert!(progress["observed_at_ms"].as_i64().unwrap() >= now);
    let error = StageAttemptError::new(
        &claim.context,
        Stage::Transcribe,
        StageErrorPhase::Adapter,
        None,
        vec![hash_bytes(b"synthetic")],
        1234,
        &OmmError {
            code: ErrorCode::Timeout,
            message: "PRIVATE_AUTH_BODY_PATH".into(),
            retryable: true,
        },
    );
    let value = serde_json::to_value(&error).unwrap();
    assert!(value["observed_at_ms"].as_i64().unwrap() >= now);
    assert!(!value.to_string().contains("PRIVATE_AUTH_BODY_PATH"));
    store.record_stage_error(&error).unwrap();
    let mut later = error.clone();
    later.elapsed_ms = 9000;
    store.record_stage_error(&later).unwrap();
    drop(store);
    let store = Store::open_read_only(&paths).unwrap();
    assert_eq!(
        store.stage_error_events(task.id).unwrap()[0].elapsed_ms,
        1234
    );
    assert!(store
        .state::<serde_json::Value>(&format!("stage-progress:{}", task.id))
        .unwrap()
        .is_some());
}

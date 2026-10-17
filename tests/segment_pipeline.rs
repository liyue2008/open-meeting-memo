use omm::core::{
    contracts::*,
    paths::*,
    store::{hash_bytes, hash_file, Store},
};
use tempfile::TempDir;
use uuid::Uuid;
fn fixture() -> (TempDir, Paths, SealedMeeting) {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let p = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
    p.create_private_dirs().unwrap();
    let id = Uuid::new_v4();
    let dir = p.data.join("meetings").join(id.to_string());
    private_dir(&dir).unwrap();
    (
        t,
        p,
        SealedMeeting {
            session_id: id,
            title: "Synthetic".into(),
            directory: dir,
            segments: vec![],
            sealed_at_ms: 0,
        },
    )
}
fn audio(m: &SealedMeeting, n: u32) -> AudioSegment {
    let path = m.directory.join(format!("segment-{n:04}.wav"));
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
    for _ in 0..4800 {
        w.write_sample(12i16).unwrap();
        w.write_sample(-12i16).unwrap();
    }
    w.finalize().unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    AudioSegment {
        capture: None,
        sha256: hash_file(&path).unwrap(),
        path,
        duration_ms: 100,
    }
}
#[test]
fn cli_empty_partial_explains_unfinished_state_without_committing_final_or_writing_database() {
    let (_t, p, m) = fixture();
    let mut store = Store::open(&p).unwrap();
    let parent = store
        .begin_segmented(&m, &ConfigSnapshot::default())
        .unwrap();
    let source = audio(&m, 0);
    store.admit_segment(m.session_id, 0, &source).unwrap();
    drop(store);
    let db = p.state.join("state.sqlite");
    let before = std::fs::read(&db).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_omm"))
        .env_clear()
        .env("HOME", _t.path().join("home"))
        .args([
            "meeting",
            "read",
            &m.session_id.to_string(),
            "--artifact",
            "transcript",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["partial"], true);
    assert_eq!(v["data"]["completedSegments"], 0);
    assert_eq!(v["data"]["totalSegments"], 1);
    assert!(v["data"]["notice"].as_str().unwrap().contains("尚未完成"));
    assert!(
        serde_json::from_str::<Transcript>(v["data"]["content"].as_str().unwrap())
            .unwrap()
            .segments
            .is_empty()
    );
    assert_eq!(std::fs::read(db).unwrap(), before);
    assert!(!Store::open_read_only(&p)
        .unwrap()
        .artifact_committed(parent.id, Stage::Transcribe)
        .unwrap());
}
fn result(model: &str) -> Transcript {
    Transcript {
        schema_version: 1,
        model_version: model.into(),
        duration_ms: 100,
        segments: vec![],
    }
}
#[test]
fn sealed_segment_claims_while_meeting_active_and_restart_keeps_checkpoint() {
    let (_t, p, mut m) = fixture();
    let cfg = ConfigSnapshot::default();
    let mut s = Store::open(&p).unwrap();
    let parent = s.begin_segmented(&m, &cfg).unwrap();
    let a = audio(&m, 0);
    s.admit_segment(m.session_id, 0, &a).unwrap();
    s.admit_segment(m.session_id, 0, &a).unwrap();
    assert!(s.claim(100, 1000).unwrap().is_none());
    let mut claim = s.claim_segment(100, 1000).unwrap().unwrap();
    assert_eq!(claim.sequence, 0);
    s.renew_segment(&mut claim, 200, 1000).unwrap();
    s.commit_segment(&claim, &result(&cfg.asr.model_version), 201)
        .unwrap();
    drop(s);
    let mut s = Store::open(&p).unwrap();
    assert!(s.claim_segment(300, 1000).unwrap().is_none());
    let b = audio(&m, 1);
    s.admit_segment(m.session_id, 1, &b).unwrap();
    let claim = s.claim_segment(300, 1000).unwrap().unwrap();
    s.fail_segment(
        &claim,
        &OmmError::new(ErrorCode::Backend, true, "synthetic failure"),
        301,
    )
    .unwrap();
    assert!(s.claim_segment(400, 1000).unwrap().is_none());
    let claim = s.claim_segment(2000, 1000).unwrap().unwrap();
    assert_eq!(claim.sequence, 1);
    assert_eq!(claim.offset_frames, 4800);
    s.commit_segment(&claim, &result(&cfg.asr.model_version), 2001)
        .unwrap();
    m.segments = vec![a, b];
    m.sealed_at_ms = 2002;
    let task = s.seal_segmented(&m).unwrap();
    assert_eq!(task.id, parent.id);
    let aggregate = s.partial_transcript(m.session_id).unwrap();
    assert_eq!(aggregate.duration_ms, 200);
    assert_eq!(s.claim(2100, 1000).unwrap().unwrap().task.id, parent.id);
}
#[test]
fn expired_segment_lease_cannot_commit_and_source_descriptor_is_immutable() {
    let (_t, p, m) = fixture();
    let mut s = Store::open(&p).unwrap();
    s.begin_segmented(&m, &ConfigSnapshot::default()).unwrap();
    let a = audio(&m, 0);
    s.admit_segment(m.session_id, 0, &a).unwrap();
    let c = s.claim_segment(100, 10).unwrap().unwrap();
    assert!(s.commit_segment(&c, &result(""), 111).is_err());
    let next = s.claim_segment(112, 1000).unwrap().unwrap();
    assert_ne!(next.context.lease.token, c.context.lease.token);
    let mut changed = a;
    changed.duration_ms += 1;
    assert!(s.admit_segment(m.session_id, 0, &changed).is_err());
}
#[tokio::test]
async fn source_chunk_progress_is_durable_before_worker_result_and_readonly_commands_do_not_write()
{
    use omm::adapters::asr::LocalAsrAdapter;
    use std::os::unix::fs::PermissionsExt;
    let (_t, p, m) = fixture();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.timeout_ms = 20_000; // Fixture startup and live-progress watchdogs, not a product deadline change.
    let mut store = Store::open(&p).unwrap();
    store.begin_segmented(&m, &cfg).unwrap();
    let audio = audio(&m, 0);
    store.admit_segment(m.session_id, 0, &audio).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let claim = store.claim_segment(now, 60000).unwrap().unwrap();
    let progress = AsrWorkerEvent::ChunkProgress {
        schema_version: 1,
        task_id: claim.context.task_id,
        attempt: 1,
        source: AudioSource::Mic,
        completed_chunks: 1,
        vad_spans: 2,
        speech_samples: 1600,
        processing_ms: 100,
        startup_timing: Some(AsrStartupTiming {
            validate_audio_ms: 1,
            validate_models_ms: 2,
            load_model_ms: 3,
            prepare_audio_ms: 4,
        }),
        chunk_timing: None,
    };
    let terminal = AsrWorkerEvent::Result {
        schema_version: 1,
        task_id: claim.context.task_id,
        attempt: 1,
        transcript: result(&cfg.asr.model_version),
    };
    let release = m.directory.join("release");
    let worker = m.directory.join("synthetic-worker");
    let producer_ready = m.directory.join("producer-ready");
    let result_written = m.directory.join("result-written");
    std::fs::write(&worker,format!("#!/bin/sh\n/bin/cat >/dev/null\n/bin/sleep 1.2\nprintf '%s\\n' '{}'\n: > '{}'\n/bin/sleep 0.15\nprintf '%s\\n' $$ > '{}'\nwhile [ ! -f '{}' ]; do /bin/sleep .01; done\nprintf '%s\\n' '{}'\nprintf done > '{}'\n",serde_json::to_string(&progress).unwrap(),producer_ready.display(),producer_ready.display(),release.display(),serde_json::to_string(&terminal).unwrap(),result_written.display())).unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adapter = LocalAsrAdapter::with_paths(worker, p.clone()).unwrap();
    let observer = async {
        let startup = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let pid = loop {
            if let Ok(line) = std::fs::read_to_string(&producer_ready) {
                if let Some(pid) = line
                    .strip_suffix('\n')
                    .and_then(|s| s.parse::<i32>().ok())
                    .filter(|p| *p > 0)
                {
                    break pid;
                }
            }
            assert!(
                std::time::Instant::now() < startup,
                "fixture producer did not flush its live progress event and complete PID frame"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            assert!(
                !release.exists() && !result_written.exists(),
                "progress must be durable before result or EOF"
            );
            assert_eq!(
                unsafe { libc::kill(pid, 0) },
                0,
                "fixture producer must remain live"
            );
            let reader = Store::open_read_only(&p).unwrap();
            if let Some(activity) = reader.segment_progress(m.session_id).unwrap()[0]
                .activity
                .as_ref()
            {
                assert!(matches!(
                    activity,
                    AsrWorkerEvent::ChunkProgress {
                        completed_chunks: 1,
                        startup_timing: Some(AsrStartupTiming {
                            load_model_ms: 3,
                            ..
                        }),
                        ..
                    }
                ));
                std::fs::write(&release, b"release").unwrap();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("progress was not parsed while worker was live")
    };
    let (out, ()) = tokio::join!(
        adapter.transcribe(AsrRequest {
            source_kind: None,
            context: claim.context.clone(),
            segments: vec![audio]
        }),
        observer
    );
    out.unwrap();
    adapter.drain().await.unwrap();
    let db = p.state.join("state.sqlite");
    let before = std::fs::read(&db).unwrap();
    let ro = Store::open_read_only(&p).unwrap();
    ro.tasks().unwrap();
    ro.meetings().unwrap();
    ro.segment_progress(m.session_id).unwrap();
    ro.partial_transcript(m.session_id).unwrap();
    ro.archive_progress(m.session_id).unwrap();
    drop(ro);
    assert_eq!(std::fs::read(db).unwrap(), before);
}
#[test]
fn failed_segment_manual_retry_never_reprocesses_valid_checkpoint_or_resets_auto_limit() {
    let (_t, p, mut m) = fixture();
    let cfg = ConfigSnapshot::default();
    let mut store = Store::open(&p).unwrap();
    let parent = store.begin_segmented(&m, &cfg).unwrap();
    let first = audio(&m, 0);
    let second = audio(&m, 1);
    store.admit_segment(m.session_id, 0, &first).unwrap();
    store.admit_segment(m.session_id, 1, &second).unwrap();
    let c = store.claim_segment(100, 1000).unwrap().unwrap();
    store
        .commit_segment(&c, &result(&cfg.asr.model_version), 101)
        .unwrap();
    let c = store.claim_segment(200, 1000).unwrap().unwrap();
    store
        .fail_segment(
            &c,
            &OmmError::new(ErrorCode::Backend, false, "synthetic fatal"),
            201,
        )
        .unwrap();
    assert!(!store
        .schedule_retry(&store.task(parent.id).unwrap(), 202)
        .unwrap());
    assert!(store.claim_segment(10000, 1000).unwrap().is_none());
    store.retry_segment_failures(m.session_id).unwrap();
    let c = store.claim_segment(11000, 1000).unwrap().unwrap();
    assert_eq!(c.sequence, 1);
    store
        .commit_segment(&c, &result(&cfg.asr.model_version), 11001)
        .unwrap();
    m.segments = vec![first, second];
    m.sealed_at_ms = 11002;
    store.seal_segmented(&m).unwrap();
    assert_eq!(
        store
            .segment_progress(m.session_id)
            .unwrap()
            .iter()
            .filter(|s| s.status == "done")
            .count(),
        2
    );
}
#[test]
fn many_non_millisecond_segment_offsets_are_derived_from_frames_without_accumulated_rounding() {
    let (_t, p, mut m) = fixture();
    let mut store = Store::open(&p).unwrap();
    let cfg = ConfigSnapshot::default();
    store.begin_segmented(&m, &cfg).unwrap();
    for n in 0..100 {
        let mut a = audio(&m, n);
        let mut w = hound::WavWriter::create(
            &a.path,
            hound::WavSpec {
                channels: 2,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..4801 {
            w.write_sample(1i16).unwrap();
            w.write_sample(2i16).unwrap();
        }
        w.finalize().unwrap();
        a.sha256 = hash_file(&a.path).unwrap();
        store.admit_segment(m.session_id, n, &a).unwrap();
        let c = store.claim_segment(100, 1000).unwrap().unwrap();
        let mut t = result(&cfg.asr.model_version);
        t.segments.push(TranscriptSegment {
            source: AudioSource::Mic,
            start_ms: 0,
            end_ms: 100,
            text: "generated".into(),
        });
        store.commit_segment(&c, &t, 101).unwrap();
        m.segments.push(a);
    }
    m.sealed_at_ms = 102;
    store.seal_segmented(&m).unwrap();
    let t = store.partial_transcript(m.session_id).unwrap();
    assert_eq!(t.duration_ms, 480100 * 1000 / 48000);
    assert_eq!(t.segments[99].start_ms, 99 * 4801 * 1000 / 48000);
    assert_eq!(t.duration_ms, 10002);
}
#[test]
fn corrupt_checkpoint_requeues_only_that_segment_before_aggregate() {
    let (_t, p, m) = fixture();
    let mut s = Store::open(&p).unwrap();
    let cfg = ConfigSnapshot::default();
    s.begin_segmented(&m, &cfg).unwrap();
    for n in 0..2 {
        s.admit_segment(m.session_id, n, &audio(&m, n)).unwrap();
        let c = s
            .claim_segment(100 + n as i64 * 100, 1000)
            .unwrap()
            .unwrap();
        s.commit_segment(&c, &result(&cfg.asr.model_version), 101 + n as i64 * 100)
            .unwrap();
    }
    let db = rusqlite::Connection::open(p.state.join("state.sqlite")).unwrap();
    let dest: String = db
        .query_row(
            "SELECT destination FROM segment_jobs WHERE session=?1 AND sequence=0",
            [m.session_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    std::fs::write(dest, b"{broken").unwrap();
    assert!(Store::open_read_only(&p)
        .unwrap()
        .partial_transcript(m.session_id)
        .is_err());
    let c = s.claim_segment(500, 1000).unwrap().unwrap();
    assert_eq!(c.sequence, 0);
    s.commit_segment(&c, &result(&cfg.asr.model_version), 501)
        .unwrap();
    assert!(s.claim_segment(600, 1000).unwrap().is_none());
    assert_eq!(s.segment_progress(m.session_id).unwrap()[1].status, "done");
}
#[test]
fn summary_lease_does_not_block_next_meetings_sealed_segment() {
    let (_t, p, mut old) = fixture();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.model_version = "fixture".into();
    let mut s = Store::open(&p).unwrap();
    old.segments = vec![audio(&old, 0)];
    old.sealed_at_ms = 50;
    let task = s.enqueue(&old, &cfg, false).unwrap();
    let c = s.claim(100, 1000).unwrap().unwrap();
    s.commit_transcript(&c, &result(&cfg.asr.model_version), 101)
        .unwrap();
    let summary = s.claim_summary(200, 1000).unwrap().unwrap();
    assert_eq!(summary.task.id, task.id);
    let mut next = old.clone();
    next.session_id = Uuid::new_v4();
    next.directory = p.data.join("meetings").join(next.session_id.to_string());
    private_dir(&next.directory).unwrap();
    next.segments.clear();
    next.sealed_at_ms = 0;
    s.begin_segmented(&next, &cfg).unwrap();
    s.admit_segment(next.session_id, 0, &audio(&next, 0))
        .unwrap();
    assert!(s.claim_segment(300, 1000).unwrap().is_some());
}
#[tokio::test]
async fn legacy_wav_database_read_only_commands_do_not_migrate_or_require_segment_tables() {
    let (_t, p, mut m) = fixture();
    m.segments = vec![audio(&m, 0)];
    m.sealed_at_ms = 1;
    let mut s = Store::open(&p).unwrap();
    let task = s.enqueue(&m, &ConfigSnapshot::default(), false).unwrap();
    drop(s);
    let dbpath = p.state.join("state.sqlite");
    let db = rusqlite::Connection::open(&dbpath).unwrap();
    db.execute_batch("DROP TABLE segment_activity;DROP TABLE segment_archives;DROP TABLE segment_jobs;DROP TABLE segmented_sessions;").unwrap();
    drop(db);
    let hash = hash_file(&dbpath).unwrap();
    let runtime = Err(OmmError::unavailable("synthetic runtime"));
    for command in [
        omm::cli::Commands::Meeting {
            command: omm::cli::MeetingCommand::Show { id: m.session_id },
        },
        omm::cli::Commands::Task {
            command: omm::cli::TaskCommand::Show { id: task.id },
        },
    ] {
        omm::cli::execute_at(&command, &p, true, &runtime)
            .await
            .unwrap();
    }
    assert_eq!(hash_file(&dbpath).unwrap(), hash);
    assert!(m.segments[0].path.exists());
}
#[test]
fn active_meeting_cannot_be_forced_into_an_early_whole_meeting_summary() {
    let (_t, p, m) = fixture();
    let mut store = Store::open(&p).unwrap();
    let cfg = ConfigSnapshot::default();
    store.begin_segmented(&m, &cfg).unwrap();
    let segment = audio(&m, 0);
    store.admit_segment(m.session_id, 0, &segment).unwrap();
    let active = store
        .meetings()
        .unwrap()
        .into_iter()
        .find(|v| v.session_id == m.session_id)
        .unwrap();
    assert_eq!(active.sealed_at_ms, 0);
    assert!(store.enqueue(&active, &cfg, true).is_err());
    assert_eq!(store.tasks().unwrap().len(), 1);
}
#[test]
fn final_frame_clock_commit_and_repeated_summary_import_do_not_sum_rounded_durations() {
    let (_t, p, mut m) = fixture();
    let mut s = Store::open(&p).unwrap();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.model_version = "synthetic-model".into();
    let parent = s.begin_segmented(&m, &cfg).unwrap();
    for n in 0..4 {
        let mut a = audio(&m, n);
        let mut writer = hound::WavWriter::append(&a.path).unwrap();
        for _ in 0..13 {
            writer.write_sample(12i16).unwrap();
            writer.write_sample(-12i16).unwrap();
        }
        writer.finalize().unwrap();
        a.sha256 = hash_file(&a.path).unwrap();
        let doc=serde_json::to_vec(&serde_json::json!({"schema_version":1,"session_id":m.session_id,"instance_id":m.session_id,"sequence":n,"offset_frames":u64::from(n)*4813,"frame_count":4813,"sample_rate":48000,"path":a.path,"temporary":true})).unwrap();
        omm::core::paths::write_private_new(
            &std::path::PathBuf::from(format!("{}.sealed.json", a.path.display())),
            &doc,
        )
        .unwrap();
        a.capture = Some(CaptureSegmentIdentity {
            session_id: m.session_id,
            instance_id: m.session_id,
            sequence: n,
            offset_frames: u64::from(n) * 4813,
            frame_count: 4813,
            sample_rate: 48000,
            descriptor_sha256: omm::core::store::hash_bytes(&doc),
            legacy_wav: false,
        });
        s.admit_segment(m.session_id, n, &a).unwrap();
        let c = s
            .claim_segment(100 + n as i64 * 10, 10000)
            .unwrap()
            .unwrap();
        s.commit_segment(&c, &result(&cfg.asr.model_version), 101 + n as i64 * 10)
            .unwrap();
        m.segments.push(a);
    }
    m.sealed_at_ms = 500;
    s.seal_segmented(&m).unwrap();
    let transcript = s.partial_transcript(m.session_id).unwrap();
    assert_eq!(transcript.duration_ms, 401);
    let claim = s.claim(500, 10000).unwrap().unwrap();
    s.commit_transcript(&claim, &transcript, 501).unwrap();
    let imported = s.force_summary(parent.id, "v2", &cfg.pi).unwrap();
    let repeated = s.force_summary(imported.id, "v3", &cfg.pi).unwrap();
    assert_eq!(
        serde_json::from_slice::<Transcript>(
            &s.read_artifact(repeated.id, Stage::Transcribe).unwrap()
        )
        .unwrap()
        .duration_ms,
        401
    );
}

#[test]
fn captured_segment_descriptor_clock_identity_and_immutable_replay_are_verified() {
    let (_t, p, m) = fixture();
    let mut s = Store::open(&p).unwrap();
    s.begin_segmented(&m, &ConfigSnapshot::default()).unwrap();
    let mut a = audio(&m, 0);
    let instance = Uuid::new_v4();
    let doc = serde_json::to_vec(&serde_json::json!({"schema_version":1,"session_id":m.session_id,"instance_id":instance,"sequence":0,"offset_frames":0,"frame_count":4800,"sample_rate":48000,"path":a.path,"temporary":true})).unwrap();
    omm::core::paths::write_private_new(
        &std::path::PathBuf::from(format!("{}.sealed.json", a.path.display())),
        &doc,
    )
    .unwrap();
    a.capture = Some(CaptureSegmentIdentity {
        session_id: m.session_id,
        instance_id: instance,
        sequence: 0,
        offset_frames: 0,
        frame_count: 4800,
        sample_rate: 48000,
        descriptor_sha256: omm::core::store::hash_bytes(&doc),
        legacy_wav: false,
    });
    for field in ["session", "sequence", "offset", "frames", "rate", "hash"] {
        let mut bad = a.clone();
        let c = bad.capture.as_mut().unwrap();
        match field {
            "session" => c.session_id = Uuid::new_v4(),
            "sequence" => c.sequence = 1,
            "offset" => c.offset_frames = 10,
            "frames" => c.frame_count = 100,
            "rate" => c.sample_rate = 16000,
            _ => c.descriptor_sha256 = "bad".into(),
        };
        assert!(s.admit_segment(m.session_id, 0, &bad).is_err(), "{field}");
    }
    s.admit_segment(m.session_id, 0, &a).unwrap();
    s.admit_segment(m.session_id, 0, &a).unwrap();
    let path = std::path::PathBuf::from(format!("{}.sealed.json", a.path.display()));
    std::fs::write(&path, b"{}").unwrap();
    assert!(
        s.admit_segment(m.session_id, 0, &a).is_err(),
        "replayed mutated descriptor was accepted"
    );
    std::fs::write(&path, &doc).unwrap();
    let mut changed = a.clone();
    changed.capture.as_mut().unwrap().instance_id = Uuid::new_v4();
    assert!(s.admit_segment(m.session_id, 0, &changed).is_err());
}

#[test]
fn rolling_force_and_repeated_summary_keep_100_fractional_segment_frame_clock() {
    let (_t, p, mut m) = fixture();
    let mut s = Store::open(&p).unwrap();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.model_version = "synthetic-model".into();
    s.begin_segmented(&m, &cfg).unwrap();
    for n in 0..100 {
        let mut a = audio(&m, n);
        let mut w = hound::WavWriter::append(&a.path).unwrap();
        w.write_sample(1i16).unwrap();
        w.write_sample(2i16).unwrap();
        w.finalize().unwrap();
        a.sha256 = hash_file(&a.path).unwrap();
        let doc=serde_json::to_vec(&serde_json::json!({"schema_version":1,"session_id":m.session_id,"instance_id":m.session_id,"sequence":n,"offset_frames":u64::from(n)*4801,"frame_count":4801,"sample_rate":48000,"path":a.path,"temporary":true})).unwrap();
        omm::core::paths::write_private_new(
            &std::path::PathBuf::from(format!("{}.sealed.json", a.path.display())),
            &doc,
        )
        .unwrap();
        a.capture = Some(CaptureSegmentIdentity {
            session_id: m.session_id,
            instance_id: m.session_id,
            sequence: n,
            offset_frames: u64::from(n) * 4801,
            frame_count: 4801,
            sample_rate: 48000,
            descriptor_sha256: omm::core::store::hash_bytes(&doc),
            legacy_wav: false,
        });
        s.admit_segment(m.session_id, n, &a).unwrap();
        let c = s
            .claim_segment(100 + n as i64 * 10, 10000)
            .unwrap()
            .unwrap();
        s.commit_segment(&c, &result(&cfg.asr.model_version), 101 + n as i64 * 10)
            .unwrap();
        m.segments.push(a);
    }
    m.sealed_at_ms = 2000;
    s.seal_segmented(&m).unwrap();
    let tr = s.partial_transcript(m.session_id).unwrap();
    assert_eq!(tr.duration_ms, 10002);
    let c = s.claim(2100, 10000).unwrap().unwrap();
    s.commit_transcript(&c, &tr, 2101).unwrap();
    let forced = s.enqueue(&m, &cfg, true).unwrap();
    let c = s.claim_asr(2200, 10000).unwrap().unwrap();
    assert_eq!(c.task.id, forced.id);
    s.commit_transcript(&c, &tr, 2201).unwrap();
    let imported = s.force_summary(forced.id, "v2", &cfg.pi).unwrap();
    let repeated = s.force_summary(imported.id, "v3", &cfg.pi).unwrap();
    let read: Transcript =
        serde_json::from_slice(&s.read_artifact(repeated.id, Stage::Transcribe).unwrap()).unwrap();
    assert_eq!(read.duration_ms, 10002);
}

#[test]
fn interrupted_legacy_wav_over_thirty_minutes_is_preserved_but_new_capture_is_rejected() {
    use std::io::{Seek, SeekFrom, Write};
    let (_temp, p, m) = fixture();
    let mut a = audio(&m, 0);
    let frames = 1800u64 * 48000 + 1;
    let mut header = std::fs::read(&a.path).unwrap();
    let data_chunk = header.windows(4).position(|b| b == b"data").unwrap();
    let data_start = data_chunk + 8;
    let bytes = frames * 4;
    header[data_chunk + 4..data_start].copy_from_slice(&(bytes as u32).to_le_bytes());
    header[4..8].copy_from_slice(&((data_start as u64 + bytes - 8) as u32).to_le_bytes());
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(&a.path)
        .unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(&header[..data_start]).unwrap();
    file.set_len(data_start as u64 + bytes).unwrap();
    drop(file);
    a.sha256 = hash_file(&a.path).unwrap();
    a.duration_ms = frames * 1000 / 48000;
    let instance = Uuid::new_v4();
    let mut capture = CaptureSegmentIdentity {
        session_id: m.session_id,
        instance_id: instance,
        sequence: 0,
        offset_frames: 0,
        frame_count: frames,
        sample_rate: 48000,
        descriptor_sha256: String::new(),
        legacy_wav: false,
    };
    let descriptor = std::path::PathBuf::from(format!("{}.sealed.json", a.path.display()));
    let mut s = Store::open(&p).unwrap();
    s.begin_segmented(&m, &ConfigSnapshot::default()).unwrap();
    for legacy in [false, true] {
        let doc = serde_json::to_vec(&serde_json::json!({"schema_version":1,"session_id":m.session_id,"instance_id":instance,"sequence":0,"offset_frames":0,"frame_count":frames,"sample_rate":48000,"path":a.path,"temporary":false,"legacy_wav":legacy})).unwrap();
        write_private_replace(&descriptor, &doc).unwrap();
        capture.legacy_wav = legacy;
        capture.descriptor_sha256 = hash_bytes(&doc);
        a.capture = Some(capture.clone());
        let admitted = s.admit_segment(m.session_id, 0, &a);
        if legacy {
            admitted.unwrap();
        } else {
            assert!(admitted.is_err());
        }
    }
    assert!(a.path.exists(), "legacy original must remain intact");
    s.admit_segment(m.session_id, 0, &a).unwrap();
}

#[derive(Clone, Copy)]
enum MaintenanceCase {
    Brief,
    Deadline,
    Business,
}
#[tokio::test]
async fn audio_maintenance_barrier_waits_without_asr_failure_notification() {
    maintenance_case(MaintenanceCase::Brief).await;
}
#[tokio::test]
async fn audio_maintenance_deadline_retains_first_error_and_notifies() {
    maintenance_case(MaintenanceCase::Deadline).await;
}
#[tokio::test]
async fn audio_maintenance_wait_does_not_suppress_real_business_failure() {
    maintenance_case(MaintenanceCase::Business).await;
}
async fn maintenance_case(case: MaintenanceCase) {
    use omm::{
        adapters::asr::LocalAsrAdapter,
        core::{notifications::*, pipeline},
    };
    use std::{
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
        sync::Arc,
    };
    struct Notices(tokio::sync::mpsc::UnboundedSender<NotificationEvent>);
    #[async_trait::async_trait]
    impl NotificationAdapter for Notices {
        async fn send(&self, e: NotificationEvent) -> Result<()> {
            self.0.send(e).unwrap();
            Ok(())
        }
    }
    struct NoSummary;
    #[async_trait::async_trait]
    impl SummaryAdapter for NoSummary {
        async fn summarize(&self, _: SummaryRequest) -> Result<Summary> {
            panic!("ASR lane only")
        }
    }
    let (_t, p, m) = fixture();
    omm::core::config::AppConfig::default().save(&p).unwrap();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.timeout_ms = if matches!(case, MaintenanceCase::Deadline) {
        500
    } else {
        20_000 // Private fake-worker startup under concurrent fixtures; the 500 ms barrier deadline case is unchanged.
    };
    let mut store = Store::open(&p).unwrap();
    let task = store.begin_segmented(&m, &cfg).unwrap();
    store.admit_segment(m.session_id, 0, &audio(&m, 0)).unwrap();
    let worker = m.directory.join("barrier-worker");
    let event = if matches!(case, MaintenanceCase::Business) {
        AsrWorkerEvent::Error {
            schema_version: 1,
            task_id: task.id,
            attempt: 1,
            error: OmmError::new(
                ErrorCode::Backend,
                false,
                "SYNTHETIC_PRIVATE_TEXT/path/secret",
            ),
        }
    } else {
        AsrWorkerEvent::Result {
            schema_version: 1,
            task_id: task.id,
            attempt: 1,
            transcript: result(&cfg.asr.model_version),
        }
    };
    std::fs::write(
        &worker,
        format!(
            "#!/bin/sh\n/bin/cat >/dev/null\nprintf '%s\\n' '{}'\n",
            serde_json::to_string(&event).unwrap()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(p.state.join("audio-readers.lock"))
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    let adapter = LocalAsrAdapter::with_paths(worker, p.clone()).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (sink, runner) = notification_channel(p.clone(), Arc::new(Notices(tx)));
    let started = std::time::Instant::now();
    let mut operation = Box::pin(pipeline::run_asr_one_notifying(
        &mut store, &adapter, &NoSummary, 60000, &sink,
    ));
    let early = tokio::time::timeout(std::time::Duration::from_millis(50), &mut operation).await;
    let waited = early.is_err();
    if matches!(case, MaintenanceCase::Deadline) {
        if early.is_err() {
            operation.as_mut().await.unwrap();
        }
        drop(lock);
    } else {
        drop(lock);
        match early {
            Ok(r) => r.unwrap(),
            Err(_) => operation.as_mut().await.unwrap(),
        };
    }
    drop(operation);
    sink.emit(NotificationEvent::RecordingStopped, task.id, 1);
    let mut notices = vec![];
    while let Some(e) = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
    {
        notices.push(e);
        if e == NotificationEvent::RecordingStopped {
            break;
        }
    }
    let progress = store.segment_progress(m.session_id).unwrap();
    eprintln!(
        "barrier waited={waited}, fixture_elapsed_ms={}, error={:?}, notices={notices:?}",
        started.elapsed().as_millis(),
        progress[0].error
    );
    assert!(
        waited,
        "maintenance must defer ASR rather than fail immediately"
    );
    let events = store.stage_error_events(task.id).unwrap();
    if matches!(case, MaintenanceCase::Brief) {
        assert!(!notices.contains(&NotificationEvent::AsrFailed));
        assert_eq!(progress[0].status, "done");
        assert!(events.is_empty());
    } else {
        assert!(notices.contains(&NotificationEvent::AsrFailed));
        assert_eq!(events.len(), 1);
        let first = &events[0];
        assert!(matches!(
            first.phase,
            omm::core::store::StageErrorPhase::Adapter
        ));
        assert_eq!(first.sequence, Some(0));
        assert_eq!(
            first.input_hashes,
            vec![hash_file(&m.directory.join("segment-0000.wav")).unwrap()]
        );
        if matches!(case, MaintenanceCase::Deadline) {
            assert_eq!(first.code, ErrorCode::Timeout);
            assert!(first.retryable);
            assert!(first.elapsed_ms >= 450);
        } else {
            assert_eq!(first.code, ErrorCode::Backend);
        }
        let bytes = serde_json::to_vec(first).unwrap();
        let mut overwritten = first.clone();
        overwritten.code = ErrorCode::Conflict;
        store.record_stage_error(&overwritten).unwrap();
        drop(store);
        let reopened = Store::open(&p).unwrap();
        assert_eq!(
            serde_json::to_vec(&reopened.stage_error_events(task.id).unwrap()[0]).unwrap(),
            bytes
        );
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains("SYNTHETIC_PRIVATE"));
    }
    runner.abort();
}

#[tokio::test]
async fn audio_maintenance_wait_cancellation_releases_file_and_starts_no_child() {
    use omm::adapters::asr::LocalAsrAdapter;
    let (_t, p, m) = fixture();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.timeout_ms = 5000;
    let mut store = Store::open(&p).unwrap();
    store.begin_segmented(&m, &cfg).unwrap();
    let a = audio(&m, 0);
    store.admit_segment(m.session_id, 0, &a).unwrap();
    let claim = store
        .claim_segment(chrono::Utc::now().timestamp_millis(), 60000)
        .unwrap()
        .unwrap();
    let adapter =
        LocalAsrAdapter::with_paths(m.directory.join("NONEXISTENT_WORKER"), p.clone()).unwrap();
    let lock = std::fs::File::create(p.state.join("audio-readers.lock")).unwrap();
    std::fs::set_permissions(
        p.state.join("audio-readers.lock"),
        std::os::unix::fs::PermissionsExt::from_mode(0o600),
    )
    .unwrap();
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    let mut op = Box::pin(adapter.transcribe(AsrRequest {
        source_kind: None,
        context: claim.context.clone(),
        segments: vec![a.clone()],
    }));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut op)
            .await
            .is_err()
    );
    drop(op);
    adapter.drain().await.unwrap();
    drop(lock);
    let file = std::fs::File::open(p.state.join("audio-readers.lock")).unwrap();
    fs2::FileExt::try_lock_exclusive(&file).unwrap();
    assert!(a.path.exists());
    assert!(store
        .recorded_asr_inputs(claim.context.task_id)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn audio_maintenance_no_candidate_does_not_open_lock_and_invalid_lock_is_not_waited() {
    let (_t, p, _m) = fixture();
    let mut store = Store::open(&p).unwrap();
    let path = p.state.join("audio-readers.lock");
    private_dir(&path).unwrap(); // Would fail open if the no-work cleanup tried to lock.
    store.cleanup_archived_sources_for_queue().unwrap();
    let e = Store::audio_read_lock_until(
        &p,
        tokio::time::Instant::now() + std::time::Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert_ne!(e.code, ErrorCode::Timeout);
}

#[tokio::test]
async fn stage_first_error_records_commit_without_content_or_paths() {
    struct InvalidResult;
    #[async_trait::async_trait]
    impl AsrAdapter for InvalidResult {
        async fn transcribe(&self, _: AsrRequest) -> Result<Transcript> {
            Ok(result("PRIVATE_INVALID_MODEL"))
        }
    }
    struct NoSummary;
    #[async_trait::async_trait]
    impl SummaryAdapter for NoSummary {
        async fn summarize(&self, _: SummaryRequest) -> Result<Summary> {
            panic!("ASR only")
        }
    }
    let (_t, p, m) = fixture();
    let mut store = Store::open(&p).unwrap();
    let task = store
        .begin_segmented(&m, &ConfigSnapshot::default())
        .unwrap();
    store.admit_segment(m.session_id, 0, &audio(&m, 0)).unwrap();
    omm::core::pipeline::run_asr_one_notifying(
        &mut store,
        &InvalidResult,
        &NoSummary,
        60000,
        &Default::default(),
    )
    .await
    .unwrap();
    let events = store.stage_error_events(task.id).unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].phase,
        omm::core::store::StageErrorPhase::Commit
    ));
    let bytes = serde_json::to_string(&events).unwrap();
    assert!(!bytes.contains("PRIVATE_INVALID_MODEL"));
    assert!(!bytes.contains(m.directory.to_str().unwrap()));
}

#[tokio::test]
async fn audio_maintenance_reader_survives_worker_cancel_until_owned_child_reaped() {
    use omm::adapters::asr::LocalAsrAdapter;
    use std::os::unix::fs::PermissionsExt;
    let (_t, p, m) = fixture();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.timeout_ms = 20_000; // Private startup handshake; the post-cancel drain deadline remains two seconds.
    let mut store = Store::open(&p).unwrap();
    store.begin_segmented(&m, &cfg).unwrap();
    let a = audio(&m, 0);
    store.admit_segment(m.session_id, 0, &a).unwrap();
    let claim = store
        .claim_segment(chrono::Utc::now().timestamp_millis(), 60000)
        .unwrap()
        .unwrap();
    let marker = m.directory.join("owned-worker-pid");
    let worker = m.directory.join("owned-worker");
    write_private_new(
        &worker,
        format!(
            "#!/bin/sh\numask 077\n: > '{}'\n/bin/sleep 0.15\nprintf '%s\\n' $$ > '{}'\nexec /bin/sleep 30\n",
            marker.display(), marker.display()
        )
        .as_bytes(),
    )
    .unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adapter = LocalAsrAdapter::with_paths(worker, p.clone()).unwrap();
    let mut op = Box::pin(adapter.transcribe(AsrRequest {
        source_kind: None,
        context: claim.context.clone(),
        segments: vec![a.clone()],
    }));
    let pid = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            tokio::select! {
                r=&mut op=>panic!("owned worker exited before cancellation: {r:?}"),
                _=tokio::time::sleep(std::time::Duration::from_millis(10))=>{
                    // Existence precedes printf completion: wait for one complete PID frame.
                    if let Ok(line) = std::fs::read_to_string(&marker) {
                        if let Some(pid) = line.strip_suffix('\n').and_then(|s| s.parse::<i32>().ok()).filter(|p| *p > 0) {
                            break pid;
                        }
                    }
                }
            }
        }
    }).await.unwrap();
    assert!(store.active_audio_readers().unwrap());
    drop(op);
    tokio::time::timeout(std::time::Duration::from_secs(2), adapter.drain())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1); // Probe only this fixture's owned child, never kill by PID.
    assert!(!store.active_audio_readers().unwrap());
    assert_eq!(
        std::fs::read_dir(&claim.context.work_dir).unwrap().count(),
        0
    );
    assert!(a.path.exists());
}

use omm::{
    adapters::asr::codec::AudioCodec,
    core::{contracts::*, paths::*, store::hash_file},
};
use uuid::Uuid;
#[tokio::test]
#[ignore = "requires explicitly built locked codec fixture"]
async fn opus_archive_is_real_mono_three_sources_exact_frames_and_keeps_wav() {
    let root = std::env::var_os("OMM_CODEC_TEST_MANIFEST").expect("set locked fixture manifest");
    let codec = AudioCodec::from_manifest(&std::path::PathBuf::from(root)).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().canonicalize().unwrap();
    let meeting = dir.join("meeting");
    let work = dir.join("work");
    private_dir(&meeting).unwrap();
    private_dir(&work).unwrap();
    let path = meeting.join("synthetic.wav");
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
    for i in 0..48013 {
        let sample = ((i as f64 * 440.0 * std::f64::consts::TAU / 48000.0).sin() * 10000.0) as i16;
        w.write_sample(sample).unwrap();
        w.write_sample(-sample).unwrap();
    }
    w.finalize().unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let audio = AudioSegment {
        capture: None,
        sha256: hash_file(&path).unwrap(),
        path: path.clone(),
        duration_ms: 1000,
    };
    let context = TaskContext {
        session_id: Uuid::new_v4(),
        task_id: Uuid::new_v4(),
        attempt: 1,
        lease: Lease {
            token: Uuid::new_v4(),
            expires_at_ms: i64::MAX,
        },
        meeting_dir: meeting,
        work_dir: work,
        config: ConfigSnapshot::default(),
    };
    let receipt = codec.archive(&context, &audio, 0, 48000).await.unwrap();
    assert_eq!(receipt.files.len(), 3);
    assert!(path.exists());
    for f in &receipt.files {
        assert_eq!(f.frames, 48013);
        assert_eq!(f.channels, 1);
        assert_eq!(std::fs::read(&f.path).unwrap()[..4], *b"OggS");
    }
    let decoded = codec.decode_sources(&receipt, &context).await.unwrap();
    let r = hound::WavReader::open(&decoded.path).unwrap();
    assert_eq!(r.duration(), 48013);
    assert_eq!(r.spec().channels, 2);
    let mut bad = receipt;
    bad.files[0].sha256 = "0".repeat(64); // mixed track does not supply ASR sources.
    bad.files[1].sha256 = "0".repeat(64);
    assert!(codec.decode_sources(&bad, &context).await.is_err());
    assert!(path.exists());
}

#[tokio::test]
#[ignore = "requires explicitly built locked codec; synthetic durable recovery only"]
async fn opus_archive_checkpoint_cleanup_requires_new_capture_provenance_and_no_reader() {
    use omm::core::store::Store;
    use std::os::unix::fs::PermissionsExt;
    let manifest = std::env::var_os("OMM_CODEC_TEST_MANIFEST").unwrap();
    let codec = AudioCodec::from_manifest(&std::path::PathBuf::from(manifest)).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
    paths.create_private_dirs().unwrap();
    let id = Uuid::new_v4();
    let dir = paths.data.join("meetings").join(id.to_string());
    private_dir(&dir).unwrap();
    let path = dir.join("retained-legacy.wav");
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
    for i in 0..48013 {
        w.write_sample((i % 200) as i16).unwrap();
        w.write_sample(0i16).unwrap();
    }
    w.finalize().unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let audio = AudioSegment {
        capture: None,
        path: path.clone(),
        sha256: hash_file(&path).unwrap(),
        duration_ms: 1000,
    };
    let cfg = ConfigSnapshot::default();
    let mut meeting = SealedMeeting {
        session_id: id,
        title: "Generated cleanup fixture".into(),
        directory: dir,
        segments: vec![],
        sealed_at_ms: 0,
    };
    let mut store = Store::open(&paths).unwrap();
    store.begin_segmented(&meeting, &cfg).unwrap();
    store.admit_segment(id, 0, &audio).unwrap();
    let archive = store.claim_archive(100, 10000).unwrap().unwrap();
    let receipt = codec
        .archive(&archive.context, &audio, 0, 48000)
        .await
        .unwrap();
    // The directory has been atomically renamed, but the database receipt is not committed.
    drop(store);
    let mut store = Store::open(&paths).unwrap();
    let resumed = store.claim_archive(10101, 10000).unwrap().unwrap();
    let recovered = codec
        .archive(&resumed.context, &audio, 0, 48000)
        .await
        .unwrap();
    assert_eq!(receipt.source_sha256, recovered.source_sha256);
    // A new lease cannot overwrite an older candidate directory; abandoned candidates are retained.
    assert_ne!(receipt.files[0].path, recovered.files[0].path);
    assert!(receipt.files.iter().all(|f| f.path.exists()));
    assert!(recovered.files.iter().all(|f| f.frames == 48013));
    assert!(store.commit_archive(&archive, &receipt, 10102).is_err());
    let mut corrupt = recovered.clone();
    corrupt.files[0].frames += 1;
    assert!(store.commit_archive(&resumed, &corrupt, 10102).is_err());
    let mut corrupt = recovered.clone();
    corrupt.files[0].pre_skip += 1;
    assert!(store.commit_archive(&resumed, &corrupt, 10102).is_err());
    let mut corrupt = recovered.clone();
    corrupt.files[0].decoded_frames += 1;
    assert!(store.commit_archive(&resumed, &corrupt, 10102).is_err());
    store.commit_archive(&resumed, &recovered, 10102).unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(path.exists());
    let segment = store.claim_segment(10103, 10000).unwrap().unwrap();
    store
        .commit_segment(
            &segment,
            &Transcript {
                schema_version: 1,
                model_version: cfg.asr.model_version.clone(),
                duration_ms: 1000,
                segments: vec![],
            },
            10104,
        )
        .unwrap();
    meeting.segments = vec![audio];
    meeting.sealed_at_ms = 10105;
    store.seal_segmented(&meeting).unwrap();
    // A legacy WAV adopted during recovery must not become disposable merely by entering a queue.
    store.cleanup_archived_sources().unwrap();
    assert!(path.exists(), "legacy WAV was deleted");
    let descriptor = std::path::PathBuf::from(format!("{}.sealed.json", path.display()));
    write_private_new(&descriptor,&serde_json::to_vec(&serde_json::json!({"schema_version":1,"sequence":0,"offset_frames":0,"frame_count":48013,"sample_rate":48000,"path":path})).unwrap()).unwrap();
    // Recovery provenance alone cannot authorize deleting a potentially pre-upgrade WAV.
    store.cleanup_archived_sources().unwrap();
    assert!(path.exists());
    std::fs::write(&descriptor, serde_json::to_vec(&serde_json::json!({"schema_version":1,"sequence":0,"offset_frames":0,"frame_count":48013,"sample_rate":48000,"path":path,"temporary":true})).unwrap()).unwrap();
    let lock = store.audio_read_lock().unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(path.exists());
    fs2::FileExt::unlock(&lock).unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(
        path.exists(),
        "an unauthenticated temporary flag deleted a legacy WAV"
    );
    drop(store);
    let mut store = Store::open(&paths).unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(store
        .archive_for_audio(id, &meeting.segments[0])
        .unwrap()
        .is_some());
    assert!(store.claim_segment(30000, 1000).unwrap().is_none());
}

#[tokio::test]
#[ignore = "requires locked codec; generated Opus force input lineage only"]
async fn opus_force_keeps_capture_clock_and_persists_actual_decoded_hash_and_codec_lineage() {
    use omm::{adapters::asr::LocalAsrAdapter, core::store::Store};
    use std::os::unix::fs::PermissionsExt;
    let manifest = std::path::PathBuf::from(std::env::var_os("OMM_CODEC_TEST_MANIFEST").unwrap());
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
    paths.create_private_dirs().unwrap();
    omm::core::audio_toolchain::install(&paths, manifest.parent().unwrap()).unwrap();
    let codec = AudioCodec::from_paths(&paths).unwrap();
    let id = Uuid::new_v4();
    let dir = paths.data.join("meetings").join(id.to_string());
    private_dir(&dir).unwrap();
    let path = dir.join("generated.wav");
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
    for i in 0..48013 {
        w.write_sample((i % 177) as i16 * 37).unwrap();
        w.write_sample((i % 133) as i16 * 29).unwrap();
    }
    w.finalize().unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let origin = Uuid::new_v4();
    let doc=serde_json::to_vec(&serde_json::json!({"schema_version":1,"session_id":id,"instance_id":origin,"sequence":0,"offset_frames":0,"frame_count":48013,"sample_rate":48000,"path":path,"temporary":true})).unwrap();
    write_private_new(
        &std::path::PathBuf::from(format!("{}.sealed.json", path.display())),
        &doc,
    )
    .unwrap();
    let audio = AudioSegment {
        sha256: hash_file(&path).unwrap(),
        path: path.clone(),
        duration_ms: 1000,
        capture: Some(CaptureSegmentIdentity {
            session_id: id,
            instance_id: origin,
            sequence: 0,
            offset_frames: 0,
            frame_count: 48013,
            sample_rate: 48000,
            descriptor_sha256: omm::core::store::hash_bytes(&doc),
            legacy_wav: false,
        }),
    };
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.timeout_ms = 30000;
    cfg.asr.model_version = omm::adapters::asr::models::MODEL_VERSION.into();
    let mut store = Store::open(&paths).unwrap();
    let mut meeting = SealedMeeting {
        session_id: id,
        title: "Generated force codec fixture".into(),
        directory: dir,
        segments: vec![],
        sealed_at_ms: 0,
    };
    store.begin_segmented(&meeting, &cfg).unwrap();
    store.admit_segment(id, 0, &audio).unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let archive = store.claim_archive(now, 60000).unwrap().unwrap();
    let receipt = codec
        .archive(&archive.context, &audio, 0, 48000)
        .await
        .unwrap();
    store.commit_archive(&archive, &receipt, now + 1).unwrap();
    meeting.segments = vec![audio.clone()];
    meeting.sealed_at_ms = now + 2;
    store.seal_segmented(&meeting).unwrap();
    let forced = store.enqueue(&meeting, &cfg, true).unwrap();
    let claim = store.claim(now + 3, 60000).unwrap().unwrap();
    assert_eq!(claim.task.id, forced.id);
    let event = AsrWorkerEvent::Result {
        schema_version: 1,
        task_id: forced.id,
        attempt: forced.attempt,
        transcript: Transcript {
            schema_version: 1,
            model_version: cfg.asr.model_version.clone(),
            duration_ms: 1000,
            segments: vec![],
        },
    };
    let script = root.join("worker.sh");
    write_private_new(
        &script,
        format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{}'\n",
            serde_json::to_string(&event).unwrap()
        )
        .as_bytes(),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adapter = LocalAsrAdapter::with_paths(script, paths.clone()).unwrap();
    let request = AsrRequest {
        source_kind: None,
        context: claim.context.clone(),
        segments: vec![audio.clone()],
    };
    use std::os::unix::fs::OpenOptionsExt;
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(paths.state.join("audio-readers.lock"))
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&lock).unwrap();
    let mut pending = Box::pin(adapter.transcribe(request.clone()));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut pending)
            .await
            .is_err()
    );
    // Only this generated fixture: maintenance removes the WAV while ASR awaits the read lock.
    std::fs::remove_file(&path).unwrap();
    drop(lock);
    assert_eq!(pending.await.unwrap().duration_ms, 1000);
    let conn = rusqlite::Connection::open(paths.state.join("state.sqlite")).unwrap();
    let document: String = conn
        .query_row(
            "SELECT document FROM asr_inputs WHERE task=?1",
            [forced.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&document).unwrap();
    let input = &value["inputs"][0];
    assert_eq!(input["original_sha256"], audio.sha256);
    assert_ne!(input["actual_sha256"], audio.sha256);
    assert_eq!(input["capture"]["frame_count"], 48013);
    assert_eq!(input["codec_lineage"]["source_sha256"], audio.sha256);
    assert_eq!(value["checkpoint_version"], 1);
    assert_eq!(value["schema_version"], 1);
    adapter.transcribe(request.clone()).await.unwrap();
    let count: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM asr_inputs WHERE task=?1",
            [forced.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert!(store.recorded_asr_inputs(forced.id).unwrap().is_some());
    // Private fake decoder with a real immutable Opus receipt: test both cancellation and the original deadline.
    let decoder = paths.data.join("audio-toolchain/ffmpeg");
    let decoder_manifest = decoder.parent().unwrap().join("manifest.json");
    let marker = claim.context.meeting_dir.join("owned-decoder-pid");
    write_private_replace(
        &decoder,
        format!(
            "#!/bin/sh\nprintf '%s' $$ > '{}'\nexec /bin/sleep 5\n",
            marker.display()
        )
        .as_bytes(),
    )
    .unwrap();
    std::fs::set_permissions(&decoder, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut manifest_value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&decoder_manifest).unwrap()).unwrap();
    manifest_value["ffmpegSha256"] = serde_json::json!(hash_file(&decoder).unwrap());
    write_private_replace(
        &decoder_manifest,
        &serde_json::to_vec(&manifest_value).unwrap(),
    )
    .unwrap();
    let mut pending = Box::pin(adapter.transcribe(AsrRequest {
        source_kind: None,
        context: claim.context.clone(),
        segments: vec![audio.clone()],
    }));
    tokio::time::timeout(std::time::Duration::from_secs(2),async {
        loop { tokio::select! {
            r=&mut pending=>panic!("decoder exited before cancellation: {r:?}"),
            _=tokio::time::sleep(std::time::Duration::from_millis(10))=>{if marker.exists(){break;}}
        }}
    }).await.unwrap();
    let pid: i32 = std::fs::read_to_string(&marker).unwrap().parse().unwrap();
    assert!(store.active_audio_readers().unwrap());
    drop(pending);
    tokio::time::timeout(std::time::Duration::from_secs(2), adapter.drain())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert!(!store.active_audio_readers().unwrap());
    assert!(!std::fs::read_dir(&claim.context.meeting_dir)
        .unwrap()
        .any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".decode-source-")));
    let mut deadline_request = AsrRequest {
        source_kind: None,
        context: claim.context.clone(),
        segments: vec![audio],
    };
    deadline_request.context.config.asr.timeout_ms = 250;
    let error = adapter.transcribe(deadline_request).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Timeout);
    assert!(error.message.contains("decoding audio"));
    adapter.drain().await.unwrap();
    assert!(!store.active_audio_readers().unwrap());
}

#[tokio::test]
#[ignore = "requires locked codec; synthetic immutable native capture cleanup"]
async fn opus_cleanup_requires_immutable_session_descriptor_and_preserves_reader_lock() {
    use omm::core::store::Store;
    use std::os::unix::fs::PermissionsExt;
    let manifest = std::env::var_os("OMM_CODEC_TEST_MANIFEST").unwrap();
    let codec = AudioCodec::from_manifest(&std::path::PathBuf::from(manifest)).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
    paths.create_private_dirs().unwrap();
    let id = Uuid::new_v4();
    let dir = paths.data.join("meetings").join(id.to_string());
    private_dir(&dir).unwrap();
    let path = dir.join("retained-legacy.wav");
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
    for i in 0..48013 {
        w.write_sample((i % 200) as i16).unwrap();
        w.write_sample(0i16).unwrap();
    }
    w.finalize().unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let origin = Uuid::new_v4();
    let descriptor = std::path::PathBuf::from(format!("{}.sealed.json", path.display()));
    let document = serde_json::to_vec(&serde_json::json!({"schema_version":1,"session_id":id,"instance_id":origin,"sequence":0,"offset_frames":0,"frame_count":48013,"sample_rate":48000,"path":path,"temporary":true,"capture_clock":{"schema_version":1,"target_rate_hz":48000,"writer_started_utc_ms":1,"elapsed_us":1100000,"format_rejects":0,"mic":{"rate_hz":44100.0,"input_frames":44112,"target_frames":48013,"rate_changes":0,"accepted_channels":1,"input_elapsed_us":1000000,"first_accepted_offset_us":10,"last_accepted_offset_us":1000010}}})).unwrap();
    write_private_new(&descriptor, &document).unwrap();
    let audio = AudioSegment {
        capture: Some(CaptureSegmentIdentity {
            session_id: id,
            instance_id: origin,
            sequence: 0,
            offset_frames: 0,
            frame_count: 48013,
            sample_rate: 48000,
            descriptor_sha256: omm::core::store::hash_bytes(&document),
            legacy_wav: false,
        }),
        path: path.clone(),
        sha256: hash_file(&path).unwrap(),
        duration_ms: 1000,
    };
    let cfg = ConfigSnapshot::default();
    let mut meeting = SealedMeeting {
        session_id: id,
        title: "Generated cleanup fixture".into(),
        directory: dir,
        segments: vec![],
        sealed_at_ms: 0,
    };
    let mut store = Store::open(&paths).unwrap();
    store.begin_segmented(&meeting, &cfg).unwrap();
    store.admit_segment(id, 0, &audio).unwrap();
    let archive = store.claim_archive(100, 10000).unwrap().unwrap();
    let receipt = codec
        .archive(&archive.context, &audio, 0, 48000)
        .await
        .unwrap();
    // The directory has been atomically renamed, but the database receipt is not committed.
    drop(store);
    let mut store = Store::open(&paths).unwrap();
    let resumed = store.claim_archive(10101, 10000).unwrap().unwrap();
    let recovered = codec
        .archive(&resumed.context, &audio, 0, 48000)
        .await
        .unwrap();
    assert_eq!(receipt.source_sha256, recovered.source_sha256);
    // A new lease cannot overwrite an older candidate directory; abandoned candidates are retained.
    assert_ne!(receipt.files[0].path, recovered.files[0].path);
    assert!(receipt.files.iter().all(|f| f.path.exists()));
    assert!(recovered.files.iter().all(|f| f.frames == 48013));
    assert!(store.commit_archive(&archive, &receipt, 10102).is_err());
    let mut corrupt = recovered.clone();
    corrupt.files[0].frames += 1;
    assert!(store.commit_archive(&resumed, &corrupt, 10102).is_err());
    let mut corrupt = recovered.clone();
    corrupt.files[0].pre_skip += 1;
    assert!(store.commit_archive(&resumed, &corrupt, 10102).is_err());
    let mut corrupt = recovered.clone();
    corrupt.files[0].decoded_frames += 1;
    assert!(store.commit_archive(&resumed, &corrupt, 10102).is_err());
    store.commit_archive(&resumed, &recovered, 10102).unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(path.exists());
    let segment = store.claim_segment(10103, 10000).unwrap().unwrap();
    store
        .commit_segment(
            &segment,
            &Transcript {
                schema_version: 1,
                model_version: cfg.asr.model_version.clone(),
                duration_ms: 1000,
                segments: vec![],
            },
            10104,
        )
        .unwrap();
    meeting.segments = vec![audio];
    meeting.sealed_at_ms = 10105;
    store.seal_segmented(&meeting).unwrap();
    let lock = store.audio_read_lock().unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(path.exists());
    fs2::FileExt::unlock(&lock).unwrap();
    // A partial persisted group is never authority to delete the source.
    let fixture_db = rusqlite::Connection::open(paths.state.join("state.sqlite")).unwrap();
    let saved: String = fixture_db
        .query_row(
            "SELECT receipt FROM segment_archives WHERE session=?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let mut partial: serde_json::Value = serde_json::from_str(&saved).unwrap();
    partial["files"].as_array_mut().unwrap().pop();
    fixture_db
        .execute(
            "UPDATE segment_archives SET receipt=?1 WHERE session=?2",
            rusqlite::params![serde_json::to_string(&partial).unwrap(), id.to_string()],
        )
        .unwrap();
    assert!(store.cleanup_archived_sources().is_err());
    assert!(path.exists());
    fixture_db
        .execute(
            "UPDATE segment_archives SET receipt=?1 WHERE session=?2",
            rusqlite::params![saved, id.to_string()],
        )
        .unwrap();
    drop(fixture_db);
    // A cleanup failure retains the complete encoded receipt and original WAV.
    let mut bad_document: serde_json::Value = serde_json::from_slice(&document).unwrap();
    bad_document["capture_clock"]["schema_version"] = serde_json::json!(2);
    std::fs::write(&descriptor, serde_json::to_vec(&bad_document).unwrap()).unwrap();
    store.cleanup_archived_sources_for_queue().unwrap();
    assert!(path.exists());
    assert_eq!(store.archive_progress(id).unwrap()[0]["status"], "failed");
    assert!(recovered
        .files
        .iter()
        .all(|f| hash_file(&f.path).unwrap() == f.sha256));
    std::fs::write(&descriptor, &document).unwrap();
    store.retry_segment_failures(id).unwrap();
    // The normal archive lane must succeed even when no encoder is available.
    let unavailable = Err(OmmError::unavailable("fixture encoder must not run"));
    assert!(
        omm::core::archive_pipeline::run_one(&mut store, &unavailable, 10000)
            .await
            .unwrap()
    );
    assert!(!path.exists());
    assert!(recovered
        .files
        .iter()
        .all(|f| hash_file(&f.path).unwrap() == f.sha256));
    assert!(
        !omm::core::archive_pipeline::run_one(&mut store, &unavailable, 10000)
            .await
            .unwrap()
    );
    assert_eq!(store.segment_progress(id).unwrap()[0].status, "done");
    drop(store);
    let mut store = Store::open(&paths).unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(store
        .archive_for_audio(id, &meeting.segments[0])
        .unwrap()
        .is_some());
    assert!(store.claim_segment(30000, 1000).unwrap().is_none());
}

#[tokio::test]
#[ignore = "requires locked codec; synthetic immutable native capture cleanup"]
async fn opus_recovered_descriptor_cleanup_keeps_original_and_next_archive_lane_alive() {
    use omm::core::store::Store;
    use std::os::unix::fs::PermissionsExt;
    let manifest = std::env::var_os("OMM_CODEC_TEST_MANIFEST").unwrap();
    let codec = AudioCodec::from_manifest(&std::path::PathBuf::from(manifest)).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
    paths.create_private_dirs().unwrap();
    let id = Uuid::new_v4();
    let dir = paths.data.join("meetings").join(id.to_string());
    private_dir(&dir).unwrap();
    let path = dir.join("retained-legacy.wav");
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
    for i in 0..48013 {
        w.write_sample((i % 200) as i16).unwrap();
        w.write_sample(0i16).unwrap();
    }
    w.finalize().unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let origin = Uuid::new_v4();
    let descriptor = std::path::PathBuf::from(format!("{}.sealed.json", path.display()));
    let document = serde_json::to_vec(&serde_json::json!({"schema_version":1,"session_id":id,"instance_id":origin,"sequence":0,"offset_frames":0,"frame_count":48013,"sample_rate":48000,"path":path,"temporary":false,"legacy_wav":false})).unwrap();
    write_private_new(&descriptor, &document).unwrap();
    let audio = AudioSegment {
        capture: Some(CaptureSegmentIdentity {
            session_id: id,
            instance_id: origin,
            sequence: 0,
            offset_frames: 0,
            frame_count: 48013,
            sample_rate: 48000,
            descriptor_sha256: omm::core::store::hash_bytes(&document),
            legacy_wav: false,
        }),
        path: path.clone(),
        sha256: hash_file(&path).unwrap(),
        duration_ms: 1000,
    };
    let cfg = ConfigSnapshot::default();
    let mut meeting = SealedMeeting {
        session_id: id,
        title: "Generated cleanup fixture".into(),
        directory: dir,
        segments: vec![],
        sealed_at_ms: 0,
    };
    let mut store = Store::open(&paths).unwrap();
    store.begin_segmented(&meeting, &cfg).unwrap();
    store.admit_segment(id, 0, &audio).unwrap();
    let archive = store.claim_archive(100, 10000).unwrap().unwrap();
    let receipt = codec
        .archive(&archive.context, &audio, 0, 48000)
        .await
        .unwrap();
    // The directory has been atomically renamed, but the database receipt is not committed.
    drop(store);
    let mut store = Store::open(&paths).unwrap();
    let resumed = store.claim_archive(10101, 10000).unwrap().unwrap();
    let recovered = codec
        .archive(&resumed.context, &audio, 0, 48000)
        .await
        .unwrap();
    assert_eq!(receipt.source_sha256, recovered.source_sha256);
    // A new lease cannot overwrite an older candidate directory; abandoned candidates are retained.
    assert_ne!(receipt.files[0].path, recovered.files[0].path);
    assert!(receipt.files.iter().all(|f| f.path.exists()));
    assert!(recovered.files.iter().all(|f| f.frames == 48013));
    assert!(store.commit_archive(&archive, &receipt, 10102).is_err());
    let mut corrupt = recovered.clone();
    corrupt.files[0].frames += 1;
    assert!(store.commit_archive(&resumed, &corrupt, 10102).is_err());
    let mut corrupt = recovered.clone();
    corrupt.files[0].pre_skip += 1;
    assert!(store.commit_archive(&resumed, &corrupt, 10102).is_err());
    let mut corrupt = recovered.clone();
    corrupt.files[0].decoded_frames += 1;
    assert!(store.commit_archive(&resumed, &corrupt, 10102).is_err());
    store.commit_archive(&resumed, &recovered, 10102).unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(path.exists());
    let segment = store.claim_segment(10103, 10000).unwrap().unwrap();
    store
        .commit_segment(
            &segment,
            &Transcript {
                schema_version: 1,
                model_version: cfg.asr.model_version.clone(),
                duration_ms: 1000,
                segments: vec![],
            },
            10104,
        )
        .unwrap();
    meeting.segments = vec![audio];
    meeting.sealed_at_ms = 10105;
    store.seal_segmented(&meeting).unwrap();
    let lock = store.audio_read_lock().unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(path.exists());
    fs2::FileExt::unlock(&lock).unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(path.exists());
    drop(store);
    let mut store = Store::open(&paths).unwrap();
    store.cleanup_archived_sources().unwrap();
    assert!(store
        .archive_for_audio(id, &meeting.segments[0])
        .unwrap()
        .is_some());
    assert!(store.claim_segment(30000, 1000).unwrap().is_none());
}

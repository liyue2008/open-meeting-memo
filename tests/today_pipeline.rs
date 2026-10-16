//! Ready meeting aggregation runs independently of unrelated audio decoders.
use async_trait::async_trait;
use omm::core::{
    contracts::*,
    notifications::NotificationSink,
    paths::*,
    pipeline,
    store::{hash_file, Store},
};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use uuid::Uuid;

fn meeting(p: &Paths) -> SealedMeeting {
    let id = Uuid::new_v4();
    let dir = p.data.join("meetings").join(id.to_string());
    private_dir(&dir).unwrap();
    SealedMeeting {
        session_id: id,
        title: "Synthetic".into(),
        directory: dir,
        segments: vec![],
        sealed_at_ms: 0,
    }
}
fn audio(m: &SealedMeeting, sequence: u32) -> AudioSegment {
    let path = m.directory.join(format!("segment-{sequence}.wav"));
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
    for _ in 0..9600 {
        w.write_sample(0i16).unwrap();
    }
    w.finalize().unwrap();
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    AudioSegment {
        sha256: hash_file(&path).unwrap(),
        path,
        duration_ms: 100,
        capture: None,
    }
}
fn transcript(model: &str) -> Transcript {
    Transcript {
        schema_version: 1,
        model_version: model.into(),
        duration_ms: 100,
        segments: vec![TranscriptSegment {
            start_ms: 0,
            end_ms: 100,
            text: "Synthetic".into(),
            source: AudioSource::Mic,
        }],
    }
}
struct HeldAsr {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    busy: AtomicBool,
    calls: AtomicUsize,
}
#[async_trait]
impl AsrAdapter for HeldAsr {
    async fn transcribe(&self, req: AsrRequest) -> Result<Transcript> {
        assert!(!self.busy.swap(true, Ordering::SeqCst));
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        self.release.notified().await;
        self.busy.store(false, Ordering::SeqCst);
        Ok(transcript(&req.context.config.asr.model_version))
    }
    async fn drain(&self) -> Result<()> {
        assert!(
            !self.busy.load(Ordering::SeqCst),
            "aggregation must not drain another lane's decoder"
        );
        Ok(())
    }
}
struct FakePi(AtomicUsize);
#[async_trait]
impl SummaryAdapter for FakePi {
    async fn summarize(&self, req: SummaryRequest) -> Result<Summary> {
        assert!(!req.transcript.segments.is_empty());
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Summary {
            schema_version: 1,
            markdown: "# Synthetic summary".into(),
            provider: req.context.config.pi.provider,
            model: req.context.config.pi.model,
            prompt_version: req.context.config.pi.prompt_version,
        })
    }
}
#[tokio::test]
async fn ready_meetings_commit_and_summarize_while_unrelated_segment_decoder_is_held() {
    let root = tempfile::tempdir().unwrap();
    let p = Paths::resolve(
        &root.path().canonicalize().unwrap().join("home"),
        &Default::default(),
    )
    .unwrap();
    p.create_private_dirs().unwrap();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.model_version = "fixture".into();
    cfg.pi.provider = "fixture".into();
    cfg.pi.model = "fixture".into();
    cfg.pi.prompt_version = "fixture".into();
    let now = chrono::Utc::now().timestamp_millis();
    let mut store = Store::open(&p).unwrap();
    let mut ready = vec![];
    for _ in 0..11 {
        let mut m = meeting(&p);
        let parent = store.begin_segmented(&m, &cfg).unwrap();
        let a = audio(&m, 0);
        store.admit_segment(m.session_id, 0, &a).unwrap();
        let claim = store.claim_segment(now, 60_000).unwrap().unwrap();
        store
            .commit_segment(&claim, &transcript(&cfg.asr.model_version), now)
            .unwrap();
        m.segments = vec![a];
        m.sealed_at_ms = now;
        store.seal_segmented(&m).unwrap();
        ready.push(parent);
    }
    // Current segments finished, but this meeting is still live: never summarize it.
    let live = meeting(&p);
    let live_parent = store.begin_segmented(&live, &cfg).unwrap();
    let a = audio(&live, 0);
    store.admit_segment(live.session_id, 0, &a).unwrap();
    let claim = store.claim_segment(now, 60_000).unwrap().unwrap();
    store
        .commit_segment(&claim, &transcript(&cfg.asr.model_version), now)
        .unwrap();
    // A sealed multi-segment meeting still has pending audio: never finalize early.
    let mut pending = meeting(&p);
    let pending_parent = store.begin_segmented(&pending, &cfg).unwrap();
    let a = audio(&pending, 0);
    store.admit_segment(pending.session_id, 0, &a).unwrap();
    let claim = store.claim_segment(now, 60_000).unwrap().unwrap();
    store
        .commit_segment(&claim, &transcript(&cfg.asr.model_version), now)
        .unwrap();
    let b = audio(&pending, 1);
    store.admit_segment(pending.session_id, 1, &b).unwrap();
    pending.segments = vec![a, b];
    pending.sealed_at_ms = now;
    store.seal_segmented(&pending).unwrap();
    let adapter = Arc::new(HeldAsr {
        started: Default::default(),
        release: Default::default(),
        busy: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    let pi = Arc::new(FakePi(AtomicUsize::new(0)));
    let worker_adapter = adapter.clone();
    let worker_pi = pi.clone();
    let paths = p.clone();
    let worker = tokio::spawn(async move {
        pipeline::run_asr_one_notifying(
            &mut Store::open(&paths).unwrap(),
            worker_adapter.as_ref(),
            worker_pi.as_ref(),
            60_000,
            &NotificationSink::default(),
        )
        .await
        .unwrap();
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        adapter.started.notified(),
    )
    .await
    .unwrap();
    for _ in 0..22 {
        pipeline::run_summary_one_notifying(
            &mut store,
            adapter.as_ref(),
            pi.as_ref(),
            60_000,
            &NotificationSink::default(),
        )
        .await
        .unwrap();
    }
    assert!(adapter.busy.load(Ordering::SeqCst));
    for parent in &ready {
        assert_eq!(
            store.task(parent.id).unwrap().status,
            TaskStatus::Completed,
            "ready parent must not wait for unrelated ASR: {:?}",
            store.task(parent.id).unwrap().error
        );
        assert!(!store
            .read_artifact(parent.id, Stage::Transcribe)
            .unwrap()
            .is_empty());
        assert!(!store
            .read_artifact(parent.id, Stage::Summary)
            .unwrap()
            .is_empty());
    }
    assert_eq!(pi.0.load(Ordering::SeqCst), 11);
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.task(live_parent.id).unwrap().status,
        TaskStatus::Queued
    );
    assert_eq!(
        store.task(pending_parent.id).unwrap().status,
        TaskStatus::Queued
    );
    assert!(store
        .read_artifact(pending_parent.id, Stage::Transcribe)
        .is_err());
    adapter.release.notify_one();
    worker.await.unwrap();
}

#[tokio::test]
async fn aggregate_commit_failure_affects_only_its_parent_and_legacy_audio_never_enters_aggregate_lane(
) {
    let root = tempfile::tempdir().unwrap();
    let p = Paths::resolve(
        &root.path().canonicalize().unwrap().join("home"),
        &Default::default(),
    )
    .unwrap();
    p.create_private_dirs().unwrap();
    let mut cfg = ConfigSnapshot {
        version: "fixture".into(),
        ..Default::default()
    };
    cfg.asr.model_version = "fixture".into();
    cfg.pi.provider = "fixture".into();
    cfg.pi.model = "fixture".into();
    cfg.pi.prompt_version = "fixture".into();
    let now = chrono::Utc::now().timestamp_millis();
    let mut store = Store::open(&p).unwrap();
    let mut parents = vec![];
    for _ in 0..2 {
        let mut m = meeting(&p);
        let parent = store.begin_segmented(&m, &cfg).unwrap();
        let a = audio(&m, 0);
        store.admit_segment(m.session_id, 0, &a).unwrap();
        let claim = store.claim_segment(now, 60_000).unwrap().unwrap();
        store
            .commit_segment(&claim, &transcript(&cfg.asr.model_version), now)
            .unwrap();
        m.segments = vec![a];
        m.sealed_at_ms = now;
        store.seal_segmented(&m).unwrap();
        parents.push(parent);
    }
    let fixture_db = rusqlite::Connection::open(p.state.join("state.sqlite")).unwrap();
    fixture_db.execute_batch(&format!("CREATE TRIGGER reject_first_aggregate BEFORE INSERT ON commits WHEN new.task='{}' BEGIN SELECT RAISE(ABORT,'synthetic commit failure'); END;", parents[0].id)).unwrap();
    let mut legacy = meeting(&p);
    legacy.segments = vec![audio(&legacy, 0)];
    legacy.sealed_at_ms = now;
    let legacy_parent = store.enqueue(&legacy, &cfg, true).unwrap();
    let adapter = HeldAsr {
        started: Default::default(),
        release: Default::default(),
        busy: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    };
    let pi = FakePi(AtomicUsize::new(0));
    for _ in 0..4 {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            pipeline::run_summary_one_notifying(
                &mut store,
                &adapter,
                &pi,
                60_000,
                &NotificationSink::default(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
    }
    let failed = store.task(parents[0].id).unwrap();
    assert_eq!(failed.status, TaskStatus::Queued);
    assert!(failed.error.as_ref().is_some_and(|e| e.retryable));
    let errors = store.stage_error_events(failed.id).unwrap();
    assert_eq!(errors.len(), 1);
    assert!(matches!(
        errors[0].phase,
        omm::core::store::StageErrorPhase::Commit
    ));
    assert!(store
        .read_artifact(parents[0].id, Stage::Transcribe)
        .is_err());
    assert_eq!(
        store.task(parents[1].id).unwrap().status,
        TaskStatus::Completed
    );
    assert!(store.read_artifact(parents[1].id, Stage::Summary).is_ok());
    assert_eq!(
        store.task(legacy_parent.id).unwrap().status,
        TaskStatus::Queued
    );
    assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
    assert_eq!(pi.0.load(Ordering::SeqCst), 1);
    assert!(store.claim_ready_aggregate(now, 60_000).unwrap().is_none());
}

#[tokio::test]
async fn concurrent_ready_parent_claims_keep_one_transcript_commit_and_one_summary() {
    let root = tempfile::tempdir().unwrap();
    let p = Paths::resolve(
        &root.path().canonicalize().unwrap().join("home"),
        &Default::default(),
    )
    .unwrap();
    p.create_private_dirs().unwrap();
    let mut cfg = ConfigSnapshot::default();
    cfg.asr.model_version = "fixture".into();
    cfg.pi.provider = "fixture".into();
    cfg.pi.model = "fixture".into();
    cfg.pi.prompt_version = "fixture".into();
    let now = chrono::Utc::now().timestamp_millis();
    let mut store = Store::open(&p).unwrap();
    let mut m = meeting(&p);
    let parent = store.begin_segmented(&m, &cfg).unwrap();
    let a = audio(&m, 0);
    store.admit_segment(m.session_id, 0, &a).unwrap();
    let claim = store.claim_segment(now, 60_000).unwrap().unwrap();
    store
        .commit_segment(&claim, &transcript(&cfg.asr.model_version), now)
        .unwrap();
    m.segments = vec![a];
    m.sealed_at_ms = now;
    store.seal_segmented(&m).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let p = p.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = Store::open(&p).unwrap();
                barrier.wait();
                store
                    .claim_ready_aggregate(chrono::Utc::now().timestamp_millis(), 60_000)
                    .unwrap()
            })
        })
        .collect();
    let claims: Vec<_> = threads
        .into_iter()
        .filter_map(|t| t.join().unwrap())
        .collect();
    assert_eq!(claims.len(), 1);
    store
        .commit_transcript(
            &claims[0],
            &transcript(&cfg.asr.model_version),
            chrono::Utc::now().timestamp_millis(),
        )
        .unwrap();
    let asr = HeldAsr {
        started: Default::default(),
        release: Default::default(),
        busy: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    };
    let pi = FakePi(AtomicUsize::new(0));
    for _ in 0..2 {
        pipeline::run_summary_one_notifying(
            &mut store,
            &asr,
            &pi,
            60_000,
            &NotificationSink::default(),
        )
        .await
        .unwrap();
    }
    assert_eq!(store.task(parent.id).unwrap().status, TaskStatus::Completed);
    assert!(store.read_artifact(parent.id, Stage::Summary).is_ok());
    assert_eq!(pi.0.load(Ordering::SeqCst), 1);
    assert_eq!(asr.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn empty_sealed_parent_is_not_claimed_or_changed() {
    let root = tempfile::tempdir().unwrap();
    let p = Paths::resolve(
        &root.path().canonicalize().unwrap().join("home"),
        &Default::default(),
    )
    .unwrap();
    p.create_private_dirs().unwrap();
    let mut store = Store::open(&p).unwrap();
    let mut m = meeting(&p);
    let parent = store
        .begin_segmented(&m, &ConfigSnapshot::default())
        .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    m.sealed_at_ms = now;
    store.seal_segmented(&m).unwrap();
    let before = serde_json::to_value(store.task(parent.id).unwrap()).unwrap();
    assert!(
        store.claim_ready_aggregate(now, 60_000).unwrap().is_none(),
        "empty sealed parent must not be leased"
    );
    assert!(
        store.claim_asr(now, 60_000).unwrap().is_none(),
        "audio lane must not reclaim an empty segmented parent either"
    );
    assert_eq!(
        serde_json::to_value(store.task(parent.id).unwrap()).unwrap(),
        before
    );
    assert!(store.read_artifact(parent.id, Stage::Transcribe).is_err());
}

#[tokio::test]
async fn missing_or_corrupt_done_checkpoint_fails_parent_without_empty_commit() {
    for missing in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let p = Paths::resolve(
            &root.path().canonicalize().unwrap().join("home"),
            &Default::default(),
        )
        .unwrap();
        p.create_private_dirs().unwrap();
        let mut store = Store::open(&p).unwrap();
        let mut m = meeting(&p);
        let cfg = ConfigSnapshot::default();
        let parent = store.begin_segmented(&m, &cfg).unwrap();
        let a = audio(&m, 0);
        store.admit_segment(m.session_id, 0, &a).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let claim = store.claim_segment(now, 60_000).unwrap().unwrap();
        store
            .commit_segment(&claim, &transcript(&cfg.asr.model_version), now)
            .unwrap();
        m.segments = vec![a];
        m.sealed_at_ms = now;
        store.seal_segmented(&m).unwrap();
        let db = rusqlite::Connection::open(p.state.join("state.sqlite")).unwrap();
        let checkpoint: String = db
            .query_row(
                "SELECT destination FROM segment_jobs WHERE session=?1",
                [m.session_id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        if missing {
            std::fs::remove_file(checkpoint).unwrap();
        } else {
            std::fs::write(checkpoint, b"{}").unwrap();
        }
        let adapter = HeldAsr {
            started: Default::default(),
            release: Default::default(),
            busy: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        };
        let pi = FakePi(AtomicUsize::new(0));
        pipeline::run_summary_one_notifying(
            &mut store,
            &adapter,
            &pi,
            60_000,
            &NotificationSink::default(),
        )
        .await
        .unwrap();
        let task = store.task(parent.id).unwrap();
        assert_ne!(task.status, TaskStatus::Completed);
        assert!(task.error.is_some());
        assert!(store.read_artifact(parent.id, Stage::Transcribe).is_err());
        assert!(store.read_artifact(parent.id, Stage::Summary).is_err());
        assert_eq!(pi.0.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 0);
        assert!(m.segments[0].path.is_file());
    }
}

//! A rejected observer insert must never replace the pipeline's business result.
use omm::core::{
    contracts::*,
    notifications::*,
    paths::*,
    pipeline,
    store::{hash_file, Store},
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tempfile::TempDir;
use uuid::Uuid;

fn fixture() -> (TempDir, Paths, SealedMeeting, Store) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
    paths.create_private_dirs().unwrap();
    omm::core::config::AppConfig::default()
        .save(&paths)
        .unwrap();
    let id = Uuid::new_v4();
    let directory = paths.data.join("meetings").join(id.to_string());
    private_dir(&directory).unwrap();
    let meeting = SealedMeeting {
        session_id: id,
        title: "Synthetic".into(),
        directory,
        segments: vec![],
        sealed_at_ms: 0,
    };
    let store = Store::open(&paths).unwrap();
    (temp, paths, meeting, store)
}
fn config() -> ConfigSnapshot {
    ConfigSnapshot {
        asr: AsrConfig {
            model_version: "fixture".into(),
            ..Default::default()
        },
        pi: PiConfig {
            provider: "fixture".into(),
            model: "fixture".into(),
            prompt_version: "fixture".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn audio(meeting: &SealedMeeting, n: u32) -> AudioSegment {
    let path = meeting.directory.join(format!("synthetic-{n}.wav"));
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
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    AudioSegment {
        capture: None,
        sha256: hash_file(&path).unwrap(),
        path,
        duration_ms: 100,
    }
}
fn reject_diagnostics(paths: &Paths) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(paths.state.join("state.sqlite")).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_diagnostic BEFORE INSERT ON settings WHEN NEW.key LIKE 'stage-error:%' BEGIN SELECT RAISE(FAIL,'PRIVATE_DIAGNOSTIC_BODY'); END;").unwrap();
    conn
}
#[derive(Clone, Copy)]
enum Behavior {
    Valid,
    Backend,
    InvalidCommit,
}
#[derive(Default)]
struct Work {
    paths: Mutex<Vec<std::path::PathBuf>>,
    drains: AtomicUsize,
}
impl Work {
    fn create(&self, context: &TaskContext) {
        write_private_new(&context.work_dir.join("owned-fixture-work"), b"synthetic").unwrap();
        self.paths.lock().unwrap().push(context.work_dir.clone());
    }
    fn assert_removed(&self) {
        for path in self.paths.lock().unwrap().iter() {
            assert!(!path.exists());
        }
    }
}
struct Asr(Behavior, Arc<Work>);
#[async_trait::async_trait]
impl AsrAdapter for Asr {
    async fn transcribe(&self, request: AsrRequest) -> Result<Transcript> {
        self.1.create(&request.context);
        if matches!(self.0, Behavior::Backend) {
            return Err(OmmError::new(
                ErrorCode::Backend,
                true,
                "synthetic backend failure",
            ));
        }
        Ok(Transcript {
            schema_version: 1,
            model_version: if matches!(self.0, Behavior::InvalidCommit) {
                "PRIVATE_INVALID_MODEL".into()
            } else {
                request.context.config.asr.model_version
            },
            duration_ms: 100,
            segments: vec![],
        })
    }
    async fn drain(&self) -> Result<()> {
        assert!(
            self.1.paths.lock().unwrap().last().unwrap().exists(),
            "drain must precede workspace deletion"
        );
        self.1.drains.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct Pi(Behavior, Arc<Work>);
#[async_trait::async_trait]
impl SummaryAdapter for Pi {
    async fn summarize(&self, request: SummaryRequest) -> Result<Summary> {
        self.1.create(&request.context);
        if matches!(self.0, Behavior::Backend) {
            return Err(OmmError::new(
                ErrorCode::Backend,
                true,
                "synthetic summary failure",
            ));
        }
        Ok(Summary {
            schema_version: if matches!(self.0, Behavior::InvalidCommit) {
                2
            } else {
                1
            },
            provider: request.context.config.pi.provider,
            model: request.context.config.pi.model,
            prompt_version: request.context.config.pi.prompt_version,
            markdown: "Synthetic summary".into(),
        })
    }
}
struct Notices(tokio::sync::mpsc::UnboundedSender<NotificationEvent>);
#[async_trait::async_trait]
impl NotificationAdapter for Notices {
    async fn send(&self, event: NotificationEvent) -> Result<()> {
        self.0.send(event).unwrap();
        Ok(())
    }
}
fn notices(
    paths: &Paths,
) -> (
    NotificationSink,
    NotificationRunner,
    tokio::sync::mpsc::UnboundedReceiver<NotificationEvent>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (sink, runner) = notification_channel(paths.clone(), Arc::new(Notices(tx)));
    (sink, runner, rx)
}
async fn events(
    sink: &NotificationSink,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<NotificationEvent>,
    task: Uuid,
) -> Vec<NotificationEvent> {
    sink.emit(NotificationEvent::RecordingStopped, task, 1);
    let mut events = vec![];
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        if event == NotificationEvent::RecordingStopped {
            break;
        }
        events.push(event);
    }
    events
}

#[tokio::test]
async fn diagnostic_insert_failure_preserves_segment_backend_retry_notification_drain_and_next_segment(
) {
    let (_t, p, m, mut store) = fixture();
    let task = store.begin_segmented(&m, &config()).unwrap();
    let source = audio(&m, 0);
    store.admit_segment(m.session_id, 0, &source).unwrap();
    let _conn = reject_diagnostics(&p);
    let work = Arc::new(Work::default());
    let (sink, runner, mut rx) = notices(&p);
    pipeline::run_asr_one_notifying(
        &mut store,
        &Asr(Behavior::Backend, work.clone()),
        &Pi(Behavior::Valid, Default::default()),
        60000,
        &sink,
    )
    .await
    .unwrap();
    let failed = store.segment_progress(m.session_id).unwrap().remove(0);
    assert_eq!(failed.status, "queued");
    assert_eq!(failed.retries, 1);
    let error = failed.error.unwrap();
    assert_eq!(error.code, ErrorCode::Backend);
    assert!(error.retryable);
    assert_eq!(error.message, "synthetic backend failure");
    assert!(store.stage_error_events(task.id).unwrap().is_empty());
    assert_eq!(work.drains.load(Ordering::SeqCst), 1);
    work.assert_removed();
    assert!(source.path.exists());
    let next = audio(&m, 1);
    store.admit_segment(m.session_id, 1, &next).unwrap();
    pipeline::run_asr_one_notifying(
        &mut store,
        &Asr(Behavior::Valid, work.clone()),
        &Pi(Behavior::Valid, Default::default()),
        60000,
        &sink,
    )
    .await
    .unwrap();
    let progress = store.segment_progress(m.session_id).unwrap();
    assert_eq!(progress[1].status, "done");
    assert_eq!(progress[0].retries, 1);
    assert_eq!(
        events(&sink, &mut rx, task.id).await,
        vec![NotificationEvent::AsrFailed]
    );
    assert_eq!(work.drains.load(Ordering::SeqCst), 2);
    work.assert_removed();
    runner.abort();
}

async fn parent_failure(stage: Stage, behavior: Behavior) {
    let (_t, p, mut m, mut store) = fixture();
    m.segments = vec![audio(&m, 0)];
    let source = m.segments[0].clone();
    let task = store.enqueue(&m, &config(), false).unwrap();
    let work = Arc::new(Work::default());
    let pi_work = Arc::new(Work::default());
    if stage == Stage::Summary {
        pipeline::run_one(
            &mut store,
            &Asr(Behavior::Valid, work.clone()),
            &Pi(Behavior::Valid, pi_work.clone()),
            60000,
        )
        .await
        .unwrap();
    }
    let original = if stage == Stage::Summary {
        Some(store.read_artifact(task.id, Stage::Transcribe).unwrap())
    } else {
        None
    };
    let _conn = reject_diagnostics(&p);
    let (sink, runner, mut rx) = notices(&p);
    let updated = pipeline::run_one_notifying(
        &mut store,
        &Asr(behavior, work.clone()),
        &Pi(behavior, pi_work.clone()),
        60000,
        &sink,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(updated.status, TaskStatus::Failed);
    assert_eq!(
        updated.error.unwrap().code,
        if matches!(behavior, Behavior::Backend) {
            ErrorCode::Backend
        } else {
            ErrorCode::Corrupt
        }
    );
    assert!(store.stage_error_events(task.id).unwrap().is_empty());
    assert_eq!(
        events(&sink, &mut rx, task.id).await,
        vec![if stage == Stage::Summary {
            NotificationEvent::SummaryFailed
        } else {
            NotificationEvent::AsrFailed
        }]
    );
    work.assert_removed();
    pi_work.assert_removed();
    assert!(source.path.exists());
    if let Some(bytes) = original {
        assert_eq!(
            store.read_artifact(task.id, Stage::Transcribe).unwrap(),
            bytes
        );
        assert!(store.read_artifact(task.id, Stage::Summary).is_err());
    }
    runner.abort();
}
#[tokio::test]
async fn diagnostic_insert_failure_preserves_legacy_asr_backend_failure() {
    parent_failure(Stage::Transcribe, Behavior::Backend).await;
}
#[tokio::test]
async fn diagnostic_insert_failure_preserves_summary_backend_failure() {
    parent_failure(Stage::Summary, Behavior::Backend).await;
}
#[tokio::test]
async fn diagnostic_insert_failure_preserves_legacy_transcript_commit_corrupt() {
    parent_failure(Stage::Transcribe, Behavior::InvalidCommit).await;
}
#[tokio::test]
async fn diagnostic_insert_failure_preserves_summary_commit_corrupt_and_original_transcript() {
    parent_failure(Stage::Summary, Behavior::InvalidCommit).await;
}
#[tokio::test]
async fn diagnostic_insert_failure_preserves_segment_commit_invalid_input_and_source() {
    let (_t, p, m, mut store) = fixture();
    let task = store.begin_segmented(&m, &config()).unwrap();
    let source = audio(&m, 0);
    store.admit_segment(m.session_id, 0, &source).unwrap();
    let _conn = reject_diagnostics(&p);
    let work = Arc::new(Work::default());
    let (sink, runner, mut rx) = notices(&p);
    pipeline::run_asr_one_notifying(
        &mut store,
        &Asr(Behavior::InvalidCommit, work.clone()),
        &Pi(Behavior::Valid, Default::default()),
        60000,
        &sink,
    )
    .await
    .unwrap();
    let failed = store.segment_progress(m.session_id).unwrap().remove(0);
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.error.unwrap().code, ErrorCode::InvalidInput);
    assert!(source.path.exists());
    assert!(store
        .partial_transcript(m.session_id)
        .unwrap()
        .segments
        .is_empty());
    assert_eq!(
        events(&sink, &mut rx, task.id).await,
        vec![NotificationEvent::AsrFailed]
    );
    work.assert_removed();
    assert_eq!(work.drains.load(Ordering::SeqCst), 1);
    runner.abort();
}
#[tokio::test]
async fn diagnostic_best_effort_does_not_swallow_business_database_failure() {
    let (_t, p, m, mut store) = fixture();
    store.begin_segmented(&m, &config()).unwrap();
    store.admit_segment(m.session_id, 0, &audio(&m, 0)).unwrap();
    let conn = reject_diagnostics(&p);
    conn.execute_batch("CREATE TRIGGER reject_business BEFORE UPDATE ON segment_jobs WHEN NEW.status='queued' AND OLD.status='running' BEGIN SELECT RAISE(FAIL,'synthetic business failure'); END;").unwrap();
    let error = pipeline::run_one(
        &mut store,
        &Asr(Behavior::Backend, Default::default()),
        &Pi(Behavior::Valid, Default::default()),
        60000,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);
    assert_eq!(
        store.segment_progress(m.session_id).unwrap()[0].status,
        "running"
    );
}

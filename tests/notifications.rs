use async_trait::async_trait;
use omm::core::{config::AppConfig, contracts::*, notifications::*, paths::Paths};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use uuid::Uuid;

#[derive(Default)]
struct FakeNotifier(Mutex<Vec<NotificationEvent>>);
#[async_trait]
impl NotificationAdapter for FakeNotifier {
    async fn send(&self, event: NotificationEvent) -> Result<()> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}
fn paths(temp: &tempfile::TempDir) -> Paths {
    let paths = Paths::resolve(&temp.path().canonicalize().unwrap(), &Default::default()).unwrap();
    paths.create_private_dirs().unwrap();
    let mut cfg = AppConfig::default();
    cfg.pi.provider = "fixture".into();
    cfg.pi.model = "fixture".into();
    cfg.save(&paths).unwrap();
    paths
}
#[tokio::test]
async fn notifications_default_compatible_filter_and_persistent_dedup() {
    let legacy: AppConfig = serde_json::from_str("{}").unwrap();
    assert!(legacy.notifications.enabled);
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let fake = Arc::new(FakeNotifier::default());
    let (sink, runner) = notification_channel(paths.clone(), fake.clone());
    let id = Uuid::new_v4();
    sink.emit(NotificationEvent::AsrFailed, id, 1);
    sink.emit(NotificationEvent::AsrFailed, id, 1);
    sink.emit(NotificationEvent::SummaryReady, id, 1);
    drop(sink);
    runner.await.unwrap();
    assert_eq!(
        *fake.0.lock().unwrap(),
        vec![
            NotificationEvent::AsrFailed,
            NotificationEvent::SummaryReady
        ]
    );
    let (sink, runner) = notification_channel(paths.clone(), fake.clone());
    sink.emit(NotificationEvent::AsrFailed, id, 1);
    sink.emit(NotificationEvent::SummaryReady, id, 1);
    drop(sink);
    runner.await.unwrap();
    assert_eq!(fake.0.lock().unwrap().len(), 2);
    let mut cfg = AppConfig::load(&paths).unwrap();
    cfg.set("notifications.enabled", "false").unwrap();
    cfg.save(&paths).unwrap();
    let (sink, runner) = notification_channel(paths, fake.clone());
    sink.emit(NotificationEvent::RecordingStarted, Uuid::new_v4(), 1);
    drop(sink);
    runner.await.unwrap();
    assert_eq!(fake.0.lock().unwrap().len(), 2);
}

struct Reject;
#[async_trait]
impl NotificationAdapter for Reject {
    async fn send(&self, _: NotificationEvent) -> Result<()> {
        Err(OmmError::unavailable("SECRET_TRANSCRIPT_TOKEN"))
    }
}
#[tokio::test]
async fn notifications_rejection_sanitized_and_queue_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let (sink, runner) = notification_channel(paths.clone(), Arc::new(Reject));
    for _ in 0..100 {
        sink.emit(NotificationEvent::RecordingFailed, Uuid::new_v4(), 1);
    }
    drop(sink);
    tokio::time::timeout(Duration::from_secs(5), runner)
        .await
        .unwrap()
        .unwrap();
    let diagnostic: serde_json::Value = omm::core::store::Store::open(&paths)
        .unwrap()
        .state("notification_diagnostic")
        .unwrap()
        .unwrap();
    assert!(!diagnostic.to_string().contains("SECRET"));
}

#[test]
fn notifications_system_command_fixed_and_isolated() {
    use omm::adapters::notification::MacNotificationAdapter;
    let command = MacNotificationAdapter::notification_command(NotificationEvent::Test);
    let command = command.as_std();
    assert_eq!(command.get_program(), "/usr/bin/osascript");
    let args: Vec<_> = command.get_args().collect();
    assert_eq!(args[0], "-e");
    assert!(args[1].to_str().unwrap().contains("item 1 of argv"));
    assert_eq!(args[2], "--");
    assert_eq!(args[3], NotificationEvent::Test.message());
    assert!(format!("{command:?}").contains("env -i"));
    assert_eq!(command.get_envs().count(), 1);
}
struct Capture {
    status: Mutex<CaptureStatus>,
    fail_start: std::sync::atomic::AtomicBool,
    fail_stop: std::sync::atomic::AtomicBool,
}
impl Default for Capture {
    fn default() -> Self {
        Self {
            status: Mutex::new(CaptureStatus {
                recording: false,
                path: None,
                ready: true,
                session_id: None,
                errors: vec![],
            }),
            fail_start: false.into(),
            fail_stop: false.into(),
        }
    }
}
#[async_trait]
impl CaptureAdapter for Capture {
    async fn windows(&self) -> Result<Vec<WindowInfo>> {
        Ok(vec![])
    }
    async fn devices(&self) -> Result<Vec<AudioDevice>> {
        Ok(vec![])
    }
    async fn status(&self) -> Result<CaptureStatus> {
        Ok(self.status.lock().unwrap().clone())
    }
    async fn start(&self, req: CaptureRequest) -> Result<CaptureStatus> {
        if self.fail_start.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(OmmError::unavailable("SECRET_START"));
        }
        std::fs::write(&req.output_path, b"synthetic").unwrap();
        *self.status.lock().unwrap() = CaptureStatus {
            recording: true,
            path: Some(req.output_path),
            ready: true,
            session_id: Some(req.context.session_id),
            errors: vec![],
        };
        self.status().await
    }
    async fn stop(&self) -> Result<CaptureStop> {
        if self.fail_stop.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(OmmError::unavailable("SECRET_STOP"));
        }
        let mut status = self.status.lock().unwrap();
        status.recording = false;
        Ok(CaptureStop {
            session_id: status.session_id,
            segments: vec![AudioSegment {
                path: status.path.clone().unwrap(),
                duration_ms: 100,
                sha256: omm::core::store::hash_bytes(b"synthetic"),
            }],
            duration_ms: 100,
        })
    }
}
struct Asr(bool);
#[async_trait]
impl AsrAdapter for Asr {
    async fn transcribe(&self, req: AsrRequest) -> Result<Transcript> {
        if self.0 {
            return Err(OmmError::new(ErrorCode::Backend, true, "SECRET_ASR"));
        }
        Ok(Transcript {
            schema_version: 1,
            model_version: req.context.config.asr.model_version,
            duration_ms: req.segments.iter().map(|s| s.duration_ms).sum(),
            segments: vec![TranscriptSegment {
                source: AudioSource::Mic,
                start_ms: 0,
                end_ms: 90,
                text: "SECRET_TRANSCRIPT".into(),
            }],
        })
    }
}
struct Pi(bool);
#[async_trait]
impl SummaryAdapter for Pi {
    async fn summarize(&self, req: SummaryRequest) -> Result<Summary> {
        if self.0 {
            return Err(OmmError::new(ErrorCode::Backend, true, "SECRET_PI"));
        }
        Ok(Summary {
            schema_version: 1,
            markdown: "# SECRET_MINUTES".into(),
            provider: req.context.config.pi.provider,
            model: req.context.config.pi.model,
            prompt_version: req.context.config.pi.prompt_version,
        })
    }
}
#[tokio::test]
async fn notifications_capture_and_commit_order_rotation_no_duplicates() {
    use omm::{
        core::{pipeline::run_one_notifying, store::Store},
        service::daemon::Controller,
    };
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let capture = Arc::new(Capture::default());
    let fake = Arc::new(FakeNotifier::default());
    let (sink, runner) = notification_channel(paths.clone(), fake.clone());
    let mut controller = Controller::new(
        paths.clone(),
        capture.clone(),
        Uuid::new_v4(),
        &AppConfig::default(),
    );
    controller.notifications = sink.clone();
    capture
        .fail_start
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(controller.start("SECRET_TITLE".into(), None).await.is_err());
    capture
        .fail_start
        .store(false, std::sync::atomic::Ordering::SeqCst);
    controller
        .start("SECRET_TITLE".into(), Some("source".into()))
        .await
        .unwrap();
    controller.rotate().await.unwrap();
    controller.reconcile().await.unwrap();
    capture
        .fail_stop
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(controller.seal(true).await.is_err());
    assert!(controller.recording().unwrap().is_some());
    capture
        .fail_stop
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let sealed = controller.seal(true).await.unwrap();
    let task = serde_json::from_value::<Task>(sealed["task"].clone()).unwrap();
    let mut store = Store::open(&paths).unwrap();
    assert_eq!(
        run_one_notifying(&mut store, &Asr(false), &Pi(false), 10000, &sink)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::TranscriptCommitted
    );
    assert_eq!(
        run_one_notifying(&mut store, &Asr(false), &Pi(false), 10000, &sink)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    assert!(store
        .read_artifact(task.id, Stage::Summary)
        .unwrap()
        .starts_with(b"# SECRET_MINUTES"));
    drop(controller);
    drop(sink);
    runner.await.unwrap();
    assert_eq!(
        *fake.0.lock().unwrap(),
        vec![
            NotificationEvent::RecordingFailed,
            NotificationEvent::RecordingStarted,
            NotificationEvent::RecordingStopped,
            NotificationEvent::SummaryReady
        ]
    );
    let payload = serde_json::to_string(&*fake.0.lock().unwrap()).unwrap();
    assert!(!payload.contains("SECRET"));
}
#[tokio::test]
async fn notifications_failed_pipeline_retry_then_success_and_stage_filter() {
    use omm::{
        core::{pipeline::run_one_notifying, store::Store},
        service::daemon::Controller,
    };
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let fake = Arc::new(FakeNotifier::default());
    let mut cfg = AppConfig::load(&paths).unwrap();
    cfg.set(
        "notifications.events",
        r#"["asr_failed","summary_failed","summary_ready"]"#,
    )
    .unwrap();
    cfg.save(&paths).unwrap();
    let (sink, runner) = notification_channel(paths.clone(), fake.clone());
    let mut controller = Controller::new(
        paths.clone(),
        Arc::new(Capture::default()),
        Uuid::new_v4(),
        &cfg,
    );
    controller.notifications = sink.clone();
    controller.start("SECRET".into(), None).await.unwrap();
    let sealed = controller.seal(true).await.unwrap();
    let task = serde_json::from_value::<Task>(sealed["task"].clone()).unwrap();
    let mut store = Store::open(&paths).unwrap();
    assert_eq!(
        run_one_notifying(&mut store, &Asr(true), &Pi(false), 10000, &sink)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Failed
    );
    store.retry(task.id).unwrap();
    run_one_notifying(&mut store, &Asr(false), &Pi(false), 10000, &sink)
        .await
        .unwrap();
    assert_eq!(
        run_one_notifying(&mut store, &Asr(false), &Pi(true), 10000, &sink)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Failed
    );
    store.retry(task.id).unwrap();
    run_one_notifying(&mut store, &Asr(false), &Pi(true), 10000, &sink)
        .await
        .unwrap();
    store.retry(task.id).unwrap();
    assert_eq!(
        run_one_notifying(&mut store, &Asr(false), &Pi(false), 10000, &sink)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    drop(controller);
    drop(sink);
    runner.await.unwrap();
    assert_eq!(
        *fake.0.lock().unwrap(),
        vec![
            NotificationEvent::AsrFailed,
            NotificationEvent::SummaryFailed,
            NotificationEvent::SummaryReady
        ]
    );
}

struct Hung {
    reapers: omm::core::process::Reapers,
    pid: Mutex<Option<u32>>,
}
#[async_trait]
impl NotificationAdapter for Hung {
    async fn send(&self, _: NotificationEvent) -> Result<()> {
        let child = tokio::process::Command::new("/bin/sleep")
            .arg("30")
            .env_clear()
            .spawn()
            .unwrap();
        *self.pid.lock().unwrap() = child.id();
        let mut child = omm::core::process::OwnedChild::new(child, self.reapers.clone(), None);
        child.wait().await?;
        Ok(())
    }
    async fn drain(&self) -> Result<()> {
        self.reapers.drain().await
    }
}
#[tokio::test]
async fn notifications_timeout_reaps_child_without_delaying_seal_or_task_success() {
    use omm::{
        core::{pipeline::run_one_notifying, store::Store},
        service::daemon::Controller,
    };
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let adapter = Arc::new(Hung {
        reapers: Default::default(),
        pid: Mutex::new(None),
    });
    let (sink, mut runner) = notification_channel(paths.clone(), adapter.clone());
    let mut controller = Controller::new(
        paths.clone(),
        Arc::new(Capture::default()),
        Uuid::new_v4(),
        &AppConfig::default(),
    );
    controller.notifications = sink.clone();
    let start = std::time::Instant::now();
    controller.start("SECRET".into(), None).await.unwrap();
    controller.seal(true).await.unwrap();
    let mut store = Store::open(&paths).unwrap();
    run_one_notifying(&mut store, &Asr(false), &Pi(false), 10000, &sink)
        .await
        .unwrap();
    assert_eq!(
        run_one_notifying(&mut store, &Asr(false), &Pi(false), 10000, &sink)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    tokio::time::sleep(Duration::from_millis(3200)).await;
    let diagnostic: serde_json::Value = store.state("notification_diagnostic").unwrap().unwrap();
    assert_eq!(diagnostic["code"], "call_timed_out");
    runner.abort();
    (&mut runner).await.unwrap_err();
    adapter.drain().await.unwrap();
    let pid = adapter.pid.lock().unwrap().unwrap();
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
}

struct InvalidPi;
#[async_trait]
impl SummaryAdapter for InvalidPi {
    async fn summarize(&self, _: SummaryRequest) -> Result<Summary> {
        Ok(Summary {
            schema_version: 1,
            markdown: "SECRET_UNCOMMITTED".into(),
            provider: "wrong-provider".into(),
            model: "wrong-model".into(),
            prompt_version: "wrong-prompt".into(),
        })
    }
}
#[tokio::test]
async fn notifications_commit_rejection_is_failure_and_unknown_never_starts() {
    use omm::{
        core::{lifecycle::Action, pipeline::run_one_notifying, store::Store},
        service::daemon::Controller,
    };
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let fake = Arc::new(FakeNotifier::default());
    let (sink, runner) = notification_channel(paths.clone(), fake.clone());
    let mut controller = Controller::new(
        paths.clone(),
        Arc::new(Capture::default()),
        Uuid::new_v4(),
        &AppConfig::default(),
    );
    controller.notifications = sink.clone();
    let candidate = MeetingObservation {
        state: MeetingState::Unknown,
        evidence: Evidence::Weak,
        observed_at_ms: 1,
        source_id: None,
        title: Some("SECRET_CANDIDATE".into()),
    };
    assert_eq!(
        controller.observation_action(&candidate, true).unwrap(),
        Action::None
    );
    assert!(controller.recording().unwrap().is_none());
    controller.start("SECRET_TITLE".into(), None).await.unwrap();
    let sealed = controller.seal(true).await.unwrap();
    let task = serde_json::from_value::<Task>(sealed["task"].clone()).unwrap();
    let mut store = Store::open(&paths).unwrap();
    run_one_notifying(&mut store, &Asr(false), &InvalidPi, 10000, &sink)
        .await
        .unwrap();
    assert_eq!(
        run_one_notifying(&mut store, &Asr(false), &InvalidPi, 10000, &sink)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Failed
    );
    assert!(store.read_artifact(task.id, Stage::Summary).is_err());
    drop(controller);
    drop(sink);
    runner.await.unwrap();
    assert_eq!(
        *fake.0.lock().unwrap(),
        vec![
            NotificationEvent::RecordingStarted,
            NotificationEvent::RecordingStopped,
            NotificationEvent::SummaryFailed
        ]
    );
    let (sink, runner) = notification_channel(paths.clone(), Arc::new(Reject));
    store.retry(task.id).unwrap();
    assert_eq!(
        run_one_notifying(&mut store, &Asr(false), &Pi(false), 10000, &sink)
            .await
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    drop(sink);
    runner.await.unwrap();
    assert_eq!(store.task(task.id).unwrap().status, TaskStatus::Completed);
    let diagnostic: serde_json::Value = store.state("notification_diagnostic").unwrap().unwrap();
    assert_eq!(diagnostic["code"], "call_rejected");
}

#[tokio::test]
async fn notifications_cli_check_does_not_claim_permission_or_hide_bad_configuration() {
    use omm::cli::{execute, Commands, NotificationCommand};
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let command = Commands::Notifications {
        command: NotificationCommand::Check,
    };
    let check = execute(&command, &paths, true).await.unwrap();
    assert_eq!(check["enabled"], true);
    assert_eq!(check["permission"], "unknown");
    assert_eq!(check["visibility"], "unknown");
    let error = execute(
        &Commands::Notifications {
            command: NotificationCommand::Request,
        },
        &paths,
        true,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InteractionRequired);
    std::fs::write(paths.config.join("config.json"), b"{INVALID_CONFIG}").unwrap();
    let check = execute(&command, &paths, true).await.unwrap();
    assert_eq!(check["configurationReadable"], false);
    assert!(check["enabled"].is_null());
    assert!(!paths.state.join("state.sqlite").exists());
}

#[tokio::test]
async fn notifications_n1_01_same_attempt_failure_after_cooldown() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let fake = Arc::new(FakeNotifier::default());
    let task = Uuid::new_v4();
    let (sink, runner) = notification_channel(paths.clone(), fake.clone());
    sink.emit(NotificationEvent::AsrFailed, task, 1);
    drop(sink);
    runner.await.unwrap();
    omm::core::store::Store::open(&paths)
        .unwrap()
        .set_state(
            "notification_throttle:\"asr_failed\"",
            &(chrono::Utc::now().timestamp_millis() - 60_001),
        )
        .unwrap();
    let (sink, runner) = notification_channel(paths, fake.clone());
    sink.emit(NotificationEvent::AsrFailed, task, 1);
    drop(sink);
    runner.await.unwrap();
    assert_eq!(
        *fake.0.lock().unwrap(),
        vec![NotificationEvent::AsrFailed; 2]
    );
}

struct Clock(std::sync::atomic::AtomicI64);
impl NotificationClock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}
impl Clock {
    fn set(&self, ms: i64) {
        self.0.store(ms, std::sync::atomic::Ordering::SeqCst);
    }
}
async fn emit_at(
    paths: &Paths,
    adapter: Arc<dyn NotificationAdapter>,
    clock: Arc<Clock>,
    events: &[(NotificationEvent, Uuid)],
) {
    let (sink, runner) = notification_channel_with_clock(paths.clone(), adapter, clock);
    for (event, scope) in events {
        sink.emit(*event, *scope, 1);
    }
    drop(sink);
    runner.await.unwrap();
}
#[tokio::test]
async fn notifications_failure_cooldown_persists_not_receipts_and_recovers_other_task() {
    use omm::core::store::Store;
    for event in [
        NotificationEvent::AsrFailed,
        NotificationEvent::SummaryFailed,
        NotificationEvent::RecordingFailed,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let fake = Arc::new(FakeNotifier::default());
        let clock = Arc::new(Clock(1_000_000.into()));
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        emit_at(
            &paths,
            fake.clone(),
            clock.clone(),
            &[(event, first), (event, first)],
        )
        .await;
        assert_eq!(*fake.0.lock().unwrap(), vec![event]);
        clock.set(1_030_000);
        emit_at(&paths, fake.clone(), clock.clone(), &[(event, second)]).await;
        let store = Store::open(&paths).unwrap();
        let name = serde_json::to_string(&event).unwrap();
        for id in [first, second] {
            assert!(store
                .state::<bool>(&format!("notification:{id}:1:{name}"))
                .unwrap()
                .is_none());
        }
        assert_eq!(
            store
                .state::<i64>(&format!("notification_throttle:{name}"))
                .unwrap(),
            Some(1_000_000)
        );
        clock.set(1_059_999);
        emit_at(&paths, fake.clone(), clock.clone(), &[(event, second)]).await;
        assert_eq!(fake.0.lock().unwrap().len(), 1);
        clock.set(1_060_000);
        emit_at(&paths, fake.clone(), clock.clone(), &[(event, second)]).await;
        clock.set(1_120_000);
        emit_at(&paths, fake.clone(), clock, &[(event, first)]).await;
        assert_eq!(*fake.0.lock().unwrap(), vec![event; 3]);
    }
}
#[tokio::test]
async fn notifications_failure_clock_rollback_and_legacy_receipts_cannot_mute_forever() {
    use omm::core::store::Store;
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let fake = Arc::new(FakeNotifier::default());
    let clock = Arc::new(Clock(1_000_000.into()));
    let id = Uuid::new_v4();
    let store = Store::open(&paths).unwrap();
    store
        .set_state(&format!("notification:{id}:1:\"asr_failed\""), &true)
        .unwrap();
    store
        .set_state("notification_throttle:\"asr_failed\"", &i64::MAX)
        .unwrap();
    emit_at(
        &paths,
        fake.clone(),
        clock.clone(),
        &[(NotificationEvent::AsrFailed, id)],
    )
    .await;
    clock.set(900_000);
    emit_at(
        &paths,
        fake.clone(),
        clock.clone(),
        &[(NotificationEvent::AsrFailed, id)],
    )
    .await;
    emit_at(
        &paths,
        fake.clone(),
        clock,
        &[(NotificationEvent::AsrFailed, id)],
    )
    .await;
    assert_eq!(
        *fake.0.lock().unwrap(),
        vec![NotificationEvent::AsrFailed; 2]
    );
}
#[tokio::test]
async fn notifications_failure_disabled_or_filtered_does_not_consume_cooldown() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let fake = Arc::new(FakeNotifier::default());
    let clock = Arc::new(Clock(1_000_000.into()));
    let id = Uuid::new_v4();
    let mut cfg = AppConfig::load(&paths).unwrap();
    cfg.notifications.enabled = false;
    cfg.save(&paths).unwrap();
    emit_at(
        &paths,
        fake.clone(),
        clock.clone(),
        &[(NotificationEvent::AsrFailed, id)],
    )
    .await;
    cfg.notifications.enabled = true;
    cfg.notifications.events = vec![NotificationEvent::SummaryFailed];
    cfg.save(&paths).unwrap();
    emit_at(
        &paths,
        fake.clone(),
        clock.clone(),
        &[
            (NotificationEvent::AsrFailed, id),
            (NotificationEvent::SummaryFailed, id),
        ],
    )
    .await;
    cfg.notifications.events.push(NotificationEvent::AsrFailed);
    cfg.save(&paths).unwrap();
    emit_at(
        &paths,
        fake.clone(),
        clock,
        &[(NotificationEvent::AsrFailed, id)],
    )
    .await;
    assert_eq!(
        *fake.0.lock().unwrap(),
        vec![
            NotificationEvent::SummaryFailed,
            NotificationEvent::AsrFailed
        ]
    );
}

#[tokio::test]
async fn notifications_real_store_retry_same_attempt_can_fail_again_after_cooldown() {
    use omm::{
        core::{pipeline::run_one_notifying, store::Store},
        service::daemon::Controller,
    };
    for stage in [Stage::Transcribe, Stage::Summary] {
        let temp = tempfile::tempdir().unwrap();
        let paths = paths(&temp);
        let mut controller = Controller::new(
            paths.clone(),
            Arc::new(Capture::default()),
            Uuid::new_v4(),
            &AppConfig::default(),
        );
        controller.start("SECRET_TITLE".into(), None).await.unwrap();
        let sealed = controller.seal(true).await.unwrap();
        let task: Task = serde_json::from_value(sealed["task"].clone()).unwrap();
        let mut store = Store::open(&paths).unwrap();
        if stage == Stage::Summary {
            run_one_notifying(
                &mut store,
                &Asr(false),
                &Pi(false),
                10000,
                &NotificationSink::default(),
            )
            .await
            .unwrap();
        }
        let fake = Arc::new(FakeNotifier::default());
        let clock = Arc::new(Clock(1_000_000.into()));
        for now in [1_000_000, 1_060_000] {
            clock.set(now);
            let (sink, runner) =
                notification_channel_with_clock(paths.clone(), fake.clone(), clock.clone());
            let failed = run_one_notifying(
                &mut store,
                &Asr(stage == Stage::Transcribe),
                &Pi(true),
                10000,
                &sink,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(failed.status, TaskStatus::Failed);
            assert_eq!(failed.id, task.id);
            assert_eq!(failed.attempt, task.attempt);
            assert_eq!(failed.stage, stage);
            drop(sink);
            runner.await.unwrap();
            store.retry(task.id).unwrap();
            assert_eq!(store.task(task.id).unwrap().attempt, task.attempt);
        }
        let event = if stage == Stage::Transcribe {
            NotificationEvent::AsrFailed
        } else {
            NotificationEvent::SummaryFailed
        };
        assert_eq!(*fake.0.lock().unwrap(), vec![event; 2]);
        assert!(!serde_json::to_string(&*fake.0.lock().unwrap())
            .unwrap()
            .contains("SECRET"));
        assert!(store.meetings().unwrap()[0].segments[0].path.exists());
    }
}
struct CountingReject(Mutex<usize>);
#[async_trait]
impl NotificationAdapter for CountingReject {
    async fn send(&self, _: NotificationEvent) -> Result<()> {
        *self.0.lock().unwrap() += 1;
        Err(OmmError::unavailable("SECRET_REJECTION"))
    }
}
#[tokio::test]
async fn notifications_failure_rejection_consumes_cooldown_not_permanent_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let reject = Arc::new(CountingReject(Mutex::new(0)));
    let clock = Arc::new(Clock(1_000_000.into()));
    let id = Uuid::new_v4();
    emit_at(
        &paths,
        reject.clone(),
        clock.clone(),
        &[(NotificationEvent::AsrFailed, id)],
    )
    .await;
    clock.set(1_030_000);
    emit_at(
        &paths,
        reject.clone(),
        clock.clone(),
        &[(NotificationEvent::AsrFailed, id)],
    )
    .await;
    assert_eq!(*reject.0.lock().unwrap(), 1);
    clock.set(1_060_000);
    emit_at(
        &paths,
        reject.clone(),
        clock,
        &[(NotificationEvent::AsrFailed, id)],
    )
    .await;
    assert_eq!(*reject.0.lock().unwrap(), 2);
    let diagnostic: serde_json::Value = omm::core::store::Store::open(&paths)
        .unwrap()
        .state("notification_diagnostic")
        .unwrap()
        .unwrap();
    assert_eq!(diagnostic["code"], "call_rejected");
    assert!(!diagnostic.to_string().contains("SECRET"));
}

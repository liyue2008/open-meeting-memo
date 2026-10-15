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
                capture: None,
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

#[tokio::test]
async fn notification_failures_persist_exact_meeting_binding_including_throttled_events() {
    use omm::core::{
        paths::{private_dir, write_private_new},
        store::{hash_bytes, Store},
    };
    let temp = tempfile::tempdir().unwrap();
    let paths = paths(&temp);
    let mut store = Store::open(&paths).unwrap();
    let mut tasks = vec![];
    for title in ["Planning \"alpha\"\nproject", "Planning \"alpha\"\nproject"] {
        let session_id = Uuid::new_v4();
        let dir = paths.data.join("meetings").join(session_id.to_string());
        private_dir(&dir).unwrap();
        let path = dir.join("audio.wav");
        write_private_new(&path, b"generated").unwrap();
        let meeting = SealedMeeting {
            session_id,
            title: title.into(),
            directory: dir,
            segments: vec![AudioSegment {
                capture: None,
                path,
                duration_ms: 60000,
                sha256: hash_bytes(b"generated"),
            }],
            sealed_at_ms: chrono::Utc::now().timestamp_millis() + tasks.len() as i64 * 86400000,
        };
        tasks.push(
            store
                .enqueue(&meeting, &ConfigSnapshot::default(), false)
                .unwrap(),
        );
    }
    let fake = Arc::new(FakeNotifier::default());
    let (sink, runner) = notification_channel(paths.clone(), fake.clone());
    for task in &tasks {
        sink.emit(NotificationEvent::AsrFailed, task.id, 1);
    }
    drop(sink);
    runner.await.unwrap();
    let db = rusqlite::Connection::open(paths.state.join("state.sqlite")).unwrap();
    let mut stmt = db
        .prepare("SELECT document FROM settings WHERE key LIKE 'notification-event:%' ORDER BY key")
        .unwrap();
    let records = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| serde_json::from_str::<serde_json::Value>(&r.unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        records.len(),
        2,
        "sent and throttled events must both remain identifiable"
    );
    let first = records
        .iter()
        .find(|r| r["task_id"] == tasks[0].id.to_string())
        .unwrap();
    let second = records
        .iter()
        .find(|r| r["task_id"] == tasks[1].id.to_string())
        .unwrap();
    for task in &tasks {
        let record = records
            .iter()
            .find(|r| r["task_id"] == task.id.to_string())
            .unwrap();
        assert_eq!(record["session_id"], task.session_id.to_string());
        assert!(record["identity"].as_str().unwrap().contains("Planning"));
        assert!(!record["identity"].as_str().unwrap().contains('\n'));
    }
    assert_eq!(first["status"], "sent");
    assert_eq!(first["visibility"], "unknown");
    assert_eq!(second["status"], "throttled");
    assert_ne!(first["identity"], second["identity"]);
}

#[test]
fn notification_payload_quote_injection_is_one_argv_and_never_script() {
    use omm::{
        adapters::notification::MacNotificationAdapter, core::notifications::NotificationPayload,
    };
    let payload = NotificationPayload {
        event: NotificationEvent::AsrFailed,
        identity: "Planning \"; do shell script \"ignored\" · 10-16 08:10（1分0秒）".into(),
        safe_reason: Some(ErrorCode::Timeout),
        retry_status: "queued".into(),
        retry_count: 1,
    };
    let cmd = MacNotificationAdapter::notification_payload_command(&payload);
    let args = cmd
        .as_std()
        .get_args()
        .map(|s| s.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(args.len(), 4);
    assert_eq!(args[2], "--");
    assert!(!args[1].contains("Planning"));
    assert_eq!(args[3], payload.message());
    assert!(args[3].contains("处理超时"));
}
#[tokio::test]
async fn notification_title_privacy_empty_fallback_and_same_name_dates_are_persisted() {
    use omm::core::{notifications::MeetingIdentity, store::Store};
    let t = tempfile::tempdir().unwrap();
    let p = paths(&t);
    let mut cfg = AppConfig::load(&p).unwrap();
    cfg.notifications.show_meeting_title = false;
    cfg.save(&p).unwrap();
    let fake = Arc::new(FakeNotifier::default());
    let (sink, runner) = notification_channel(p.clone(), fake.clone());
    let hidden = Uuid::new_v4();
    sink.emit_meeting(
        NotificationEvent::RecordingStarted,
        MeetingIdentity {
            session_id: Some(hidden),
            title: "Hidden project".into(),
            started_at_ms: Some(1_700_000_000_000),
            duration_ms: Some(60000),
            time_source: MeetingTimeSource::Exact,
        },
    );
    drop(sink);
    runner.await.unwrap();
    let record = Store::open(&p)
        .unwrap()
        .notification_history(Some(hidden), 20)
        .unwrap()
        .pop()
        .unwrap();
    assert!(!record.identity.contains("Hidden"));
    assert!(record.identity.contains("开始的会议"));
    assert!(record.identity.contains("1分0秒"));
    cfg.notifications.show_meeting_title = true;
    cfg.save(&p).unwrap();
    let (sink, runner) = notification_channel(p.clone(), fake);
    let mut ids = vec![];
    for (i, title) in ["Meeting", "", "Same name\nproject", "Same name\nproject"]
        .into_iter()
        .enumerate()
    {
        let id = Uuid::new_v4();
        ids.push(id);
        sink.emit_meeting(
            NotificationEvent::RecordingStarted,
            MeetingIdentity {
                session_id: Some(id),
                title: title.into(),
                started_at_ms: Some(1_700_000_000_000 + i as i64 * 86400000),
                duration_ms: None,
                time_source: MeetingTimeSource::Exact,
            },
        );
    }
    drop(sink);
    runner.await.unwrap();
    let s = Store::open_read_only(&p).unwrap();
    let records = ids
        .into_iter()
        .map(|id| s.notification_history(Some(id), 20).unwrap().pop().unwrap())
        .collect::<Vec<_>>();
    assert!(records[0].identity.contains("开始的会议"));
    assert!(records[1].identity.contains("开始的会议"));
    assert_ne!(records[2].identity, records[3].identity);
    assert!(!records[2].identity.contains('\n'));
    assert!(s.notification_history(None, 101).is_err());
}

#[tokio::test]
async fn notification_retry_snapshot_uses_queued_state_and_three_retry_limit() {
    use omm::{
        core::{
            notifications::NotificationPayload,
            store::{StageErrorPhase, Store},
        },
        service::daemon::Controller,
    };
    let t = tempfile::tempdir().unwrap();
    let p = paths(&t);
    let mut controller = Controller::new(
        p.clone(),
        Arc::new(Capture::default()),
        Uuid::new_v4(),
        &AppConfig::default(),
    );
    controller
        .start("Generated retry meeting".into(), None)
        .await
        .unwrap();
    let sealed = controller.seal(true).await.unwrap();
    let task: Task = serde_json::from_value(sealed["task"].clone()).unwrap();
    let fake = Arc::new(FakeNotifier::default());
    let clock = Arc::new(Clock(1_000_000.into()));
    let mut store = Store::open(&p).unwrap();
    for attempt in 0..4 {
        let now = 10000 + attempt * 100000;
        let claim = store.claim(now, 10000).unwrap().unwrap();
        let error = OmmError::new(ErrorCode::Timeout, true, "PRIVATE_ERROR_BODY /private/path");
        store.fail(&claim, &error, now + 1).unwrap();
        store
            .schedule_retry(&store.task(task.id).unwrap(), now + 1)
            .unwrap();
        let retry = store
            .notification_retry_state(task.id, Stage::Transcribe, None, true)
            .unwrap();
        assert_eq!(retry.1, (attempt as u32 + 1).min(3));
        assert_eq!(retry.0, if attempt < 3 { "queued" } else { "exhausted" });
        clock.set(1_000_000 + attempt * 60000);
        let (sink, runner) =
            notification_channel_with_clock(p.clone(), fake.clone(), clock.clone());
        sink.emit_failure(
            NotificationEvent::AsrFailed,
            &claim.context,
            None,
            &error,
            StageErrorPhase::Adapter,
            Some(retry),
        );
        drop(sink);
        runner.await.unwrap();
    }
    let history = store
        .notification_history(Some(task.session_id), 20)
        .unwrap();
    assert_eq!(history.len(), 4);
    let exhausted = history
        .iter()
        .find(|r| r.retry_status == "exhausted")
        .unwrap();
    assert_eq!(exhausted.retry_count, 3);
    assert!(exhausted.lease.is_some());
    let text = NotificationPayload {
        event: exhausted.event,
        identity: exhausted.identity.clone(),
        safe_reason: exhausted.safe_reason,
        retry_status: exhausted.retry_status.clone(),
        retry_count: exhausted.retry_count,
    }
    .message();
    assert!(!text.contains("重试已排队"));
    assert!(text.contains("次数已用尽"));
    assert!(!serde_json::to_string(&history)
        .unwrap()
        .contains("PRIVATE_ERROR_BODY"));
    assert!(!text.contains(&task.id.to_string()));
}

#[tokio::test]
async fn rn1_lane_retry_snapshot_is_frozen_after_schedule_and_manual_remains_pending() {
    use omm::{
        core::{
            pipeline::{run_asr_one_notifying, run_one_notifying, run_summary_one_notifying},
            store::Store,
        },
        service::daemon::Controller,
    };
    for stage in [Stage::Transcribe, Stage::Summary] {
        let t = tempfile::tempdir().unwrap();
        let p = paths(&t);
        let mut c = Controller::new(
            p.clone(),
            Arc::new(Capture::default()),
            Uuid::new_v4(),
            &AppConfig::default(),
        );
        c.start("Generated retry snapshot".into(), None)
            .await
            .unwrap();
        let task: Task =
            serde_json::from_value(c.seal(true).await.unwrap()["task"].clone()).unwrap();
        let mut store = Store::open(&p).unwrap();
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
        let (sink, runner) = notification_channel(p.clone(), Arc::new(FakeNotifier::default()));
        // First manual failure schedules nothing. Delay the observer until later claims complete.
        let failed = run_one_notifying(&mut store, &Asr(true), &Pi(true), 10000, &sink)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.status, TaskStatus::Failed);
        store.retry(task.id).unwrap();
        for count in 1..=4 {
            let task = if stage == Stage::Transcribe {
                run_asr_one_notifying(&mut store, &Asr(true), &Pi(true), 10000, &sink)
                    .await
                    .unwrap()
                    .unwrap()
            } else {
                run_summary_one_notifying(&mut store, &Asr(true), &Pi(true), 10000, &sink)
                    .await
                    .unwrap()
                    .unwrap()
            };
            assert_eq!(
                task.status,
                if count <= 3 {
                    if stage == Stage::Transcribe {
                        TaskStatus::Queued
                    } else {
                        TaskStatus::TranscriptCommitted
                    }
                } else {
                    TaskStatus::Failed
                }
            );
            if count <= 3 {
                let db = rusqlite::Connection::open(p.state.join("state.sqlite")).unwrap();
                db.execute(
                    "UPDATE retries SET next_at=0 WHERE task=?1",
                    [task.id.to_string()],
                )
                .unwrap();
            }
        }
        drop(c);
        drop(sink);
        runner.await.unwrap();
        let history = store
            .notification_history(Some(task.session_id), 20)
            .unwrap();
        assert_eq!(history.len(), 5);
        assert!(history
            .iter()
            .any(|r| r.retry_status == "pending" && r.retry_count == 0));
        for count in 1..=3 {
            assert!(history
                .iter()
                .any(|r| r.retry_status == "queued" && r.retry_count == count));
        }
        assert!(history
            .iter()
            .any(|r| r.retry_status == "exhausted" && r.retry_count == 3));
        let leases = history
            .iter()
            .map(|r| r.lease.as_ref().unwrap().token)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(leases.len(), 5);
        for r in history {
            assert_eq!(r.stage, Some(stage));
            let error = store
                .state::<omm::core::store::StageAttemptError>(&format!(
                    "stage-error:{}",
                    r.lease.unwrap().token
                ))
                .unwrap()
                .unwrap();
            assert_eq!(error.observed_at_ms, r.first_error_at_ms);
        }
    }
}

#[tokio::test]
async fn rn1_unknown_helper_failure_has_no_fabricated_meeting_and_legacy_time_is_estimated() {
    use omm::{core::store::Store, service::daemon::Controller};
    let t = tempfile::tempdir().unwrap();
    let p = paths(&t);
    let instance = Uuid::new_v4();
    let mut c = Controller::new(
        p.clone(),
        Arc::new(Capture::default()),
        instance,
        &AppConfig::default(),
    );
    let (sink, runner) = notification_channel(p.clone(), Arc::new(FakeNotifier::default()));
    c.notifications = sink;
    c.recording_failed(&OmmError::unavailable("PRIVATE_HELPER_ERROR"));
    drop(c);
    runner.await.unwrap();
    let r = Store::open(&p)
        .unwrap()
        .notification_history(None, 20)
        .unwrap()
        .pop()
        .unwrap();
    assert!(r.session_id.is_none());
    assert!(r.identity.contains("会议时间未知"));
    assert!(!r.identity.contains("开始的会议"));
    assert!(r.occurred_at_ms > 0);
    assert!(serde_json::to_value(&r).unwrap()["meeting_started_at_ms"].is_null());
}

#[tokio::test]
async fn rn1_legacy_estimated_missing_meeting_and_old_history_are_readonly_compatible() {
    use omm::core::{
        notifications::MeetingTimeSource,
        paths::{private_dir, write_private_new},
        store::{hash_bytes, Store},
    };
    let t = tempfile::tempdir().unwrap();
    let p = paths(&t);
    let mut store = Store::open(&p).unwrap();
    let session = Uuid::new_v4();
    let dir = p.data.join("meetings").join(session.to_string());
    private_dir(&dir).unwrap();
    let audio = dir.join("generated.wav");
    write_private_new(&audio, b"generated").unwrap();
    let m = SealedMeeting {
        session_id: session,
        title: "Legacy generated".into(),
        directory: dir,
        segments: vec![AudioSegment {
            path: audio,
            duration_ms: 60000,
            sha256: hash_bytes(b"generated"),
            capture: None,
        }],
        sealed_at_ms: 1_700_000_000_000,
    };
    let task = store
        .enqueue(&m, &ConfigSnapshot::default(), false)
        .unwrap();
    let (sink, runner) = notification_channel(p.clone(), Arc::new(FakeNotifier::default()));
    omm::core::pipeline::run_one_notifying(&mut store, &Asr(true), &Pi(false), 10000, &sink)
        .await
        .unwrap();
    drop(sink);
    runner.await.unwrap();
    let record = store
        .notification_history(Some(session), 20)
        .unwrap()
        .pop()
        .unwrap();
    assert!(record.identity.contains('约'));
    assert_eq!(record.meeting_started_at_ms, Some(m.sealed_at_ms - 60000));
    assert!(matches!(
        record.meeting_time_source,
        MeetingTimeSource::Estimated
    ));
    store.retry(task.id).unwrap();
    let claim = store
        .claim(chrono::Utc::now().timestamp_millis(), 10000)
        .unwrap()
        .unwrap();
    let error = OmmError::new(ErrorCode::Timeout, true, "PRIVATE_OLD_BODY");
    store
        .fail(&claim, &error, chrono::Utc::now().timestamp_millis())
        .unwrap();
    let db = rusqlite::Connection::open(p.state.join("state.sqlite")).unwrap();
    // Model missing legacy metadata without changing the already trusted failed-claim binding.
    db.pragma_update(None, "foreign_keys", false).unwrap();
    db.execute("DELETE FROM meetings WHERE id=?1", [session.to_string()])
        .unwrap();
    let (sink, runner) = notification_channel(p.clone(), Arc::new(FakeNotifier::default()));
    sink.emit_failure(
        NotificationEvent::AsrFailed,
        &claim.context,
        None,
        &error,
        omm::core::store::StageErrorPhase::Adapter,
        Some(("pending".into(), 0, None)),
    );
    drop(sink);
    runner.await.unwrap();
    let missing = store
        .notification_history(Some(session), 20)
        .unwrap()
        .into_iter()
        .find(|r| {
            r.lease
                .as_ref()
                .is_some_and(|l| l.token == claim.lease.token)
        })
        .unwrap();
    assert_eq!(missing.session_id, Some(session));
    assert!(missing.identity.contains("会议时间未知"));
    assert!(missing.meeting_started_at_ms.is_none());
    let mut old = serde_json::to_value(record).unwrap();
    for key in [
        "identity",
        "meeting_time_source",
        "meeting_started_at_ms",
        "first_error_at_ms",
        "retry_next_at_ms",
    ] {
        old.as_object_mut().unwrap().remove(key);
    }
    store
        .set_state("notification-event:99999999999999999999:legacy", &old)
        .unwrap();
    let before:String=db.query_row("SELECT document FROM settings WHERE key='notification-event:99999999999999999999:legacy'",[],|r|r.get(0)).unwrap();
    let read = Store::open_read_only(&p)
        .unwrap()
        .notification_history(Some(session), 20)
        .unwrap();
    assert!(read.iter().any(|r| r.identity == "会议时间未知"
        && matches!(r.meeting_time_source, MeetingTimeSource::Unknown)));
    let after:String=db.query_row("SELECT document FROM settings WHERE key='notification-event:99999999999999999999:legacy'",[],|r|r.get(0)).unwrap();
    assert_eq!(before, after);
}

async fn rn2_sealed_controller_fixture() -> (tempfile::TempDir, Paths, Task, i64) {
    use omm::{core::store::Store, service::daemon::Controller};
    let temp = tempfile::tempdir().unwrap();
    let p = paths(&temp);
    let mut controller = Controller::new(
        p.clone(),
        Arc::new(Capture::default()),
        Uuid::new_v4(),
        &AppConfig::default(),
    );
    controller
        .start("Generated persistent title".into(), None)
        .await
        .unwrap();
    let mut record = controller.recording().unwrap().unwrap();
    record.started_at_ms -= 90 * 60 * 1000;
    let started = record.started_at_ms;
    let store = Store::open(&p).unwrap();
    store.set_state("recording", &record).unwrap();
    let task: Task =
        serde_json::from_value(controller.seal(true).await.unwrap()["task"].clone()).unwrap();
    assert!(store
        .state::<serde_json::Value>("recording")
        .unwrap()
        .is_none());
    store
        .clear_state(&format!("notification-meeting:{}", task.session_id))
        .unwrap();
    (temp, p, task, started)
}

#[tokio::test]
async fn rn2_persistent_meeting_start_survives_seal_cache_loss_and_ninety_minute_gap() {
    use omm::core::{pipeline::run_asr_one_notifying, store::Store};
    let (_temp, p, task, started) = rn2_sealed_controller_fixture().await;
    let mut store = Store::open(&p).unwrap();
    let meeting = store.meeting(task.session_id).unwrap().unwrap();
    let bytes = std::fs::read(meeting.directory.join("meeting.json")).unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(metadata["started_at_ms"], started);
    let (sink, runner) = notification_channel(p.clone(), Arc::new(FakeNotifier::default()));
    let queued = run_asr_one_notifying(&mut store, &Asr(true), &Pi(false), 10000, &sink)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued.status, TaskStatus::Queued);
    sink.emit_meeting(
        NotificationEvent::RecordingStopped,
        MeetingIdentity {
            session_id: Some(task.session_id),
            title: String::new(),
            started_at_ms: None,
            duration_ms: None,
            time_source: MeetingTimeSource::Unknown,
        },
    );
    sink.emit(NotificationEvent::SummaryFailed, task.id, 1);
    drop(sink);
    runner.await.unwrap();
    let history = store
        .notification_history(Some(task.session_id), 20)
        .unwrap();
    assert_eq!(history.len(), 3);
    for record in history {
        assert_eq!(record.meeting_started_at_ms, Some(started));
        assert!(matches!(
            record.meeting_time_source,
            MeetingTimeSource::Exact
        ));
        assert!(record.identity.contains("Generated persistent title"));
        assert!(!record.identity.contains('约'));
        assert!(!record.identity.contains(&task.id.to_string()));
    }
    let human = std::process::Command::new(env!("CARGO_BIN_EXE_omm"))
        .env_clear()
        .env("HOME", _temp.path())
        .args([
            "notifications",
            "history",
            "--meeting",
            &task.session_id.to_string(),
        ])
        .output()
        .unwrap();
    assert!(human.status.success());
    let human = String::from_utf8(human.stdout).unwrap();
    let time = chrono::DateTime::from_timestamp_millis(started)
        .unwrap()
        .with_timezone(&chrono::Local)
        .format("%m-%d %H:%M")
        .to_string();
    assert!(human.contains(&time));
    assert!(human.contains("Generated persistent title"));
    assert!(!human.contains('约'));
    assert!(!human.contains(&task.id.to_string()));
    assert_eq!(
        std::fs::read(meeting.directory.join("meeting.json")).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn rn2_unsafe_or_missing_persistent_metadata_is_estimated_without_stopping_business() {
    use omm::core::{pipeline::run_asr_one_notifying, store::Store};
    use std::os::unix::fs::{symlink, PermissionsExt};
    for mode in [
        "missing",
        "corrupt",
        "large",
        "symlink",
        "session",
        "sealed_session",
        "directory",
        "schema",
        "timestamp",
        "permissions",
    ] {
        let (_temp, p, task, started) = rn2_sealed_controller_fixture().await;
        let mut store = Store::open(&p).unwrap();
        let meeting = store.meeting(task.session_id).unwrap().unwrap();
        let file = meeting.directory.join("meeting.json");
        let original = std::fs::read(&file).unwrap();
        let mut doc: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match mode {
            "missing" => std::fs::remove_file(&file).unwrap(),
            "corrupt" => std::fs::write(&file, b"PRIVATE_BAD_METADATA").unwrap(),
            "large" => std::fs::write(&file, vec![b'x'; 262145]).unwrap(),
            "symlink" => {
                let other = p.data.join("private-other.json");
                omm::core::paths::write_private_new(&other, &original).unwrap();
                std::fs::remove_file(&file).unwrap();
                symlink(other, &file).unwrap();
            }
            "permissions" => {
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap()
            }
            _ => {
                match mode {
                    "session" => doc["session_id"] = serde_json::json!(Uuid::new_v4()),
                    "sealed_session" => {
                        doc["sealed"]["session_id"] = serde_json::json!(Uuid::new_v4())
                    }
                    "directory" => doc["directory"] = serde_json::json!(p.data),
                    "schema" => doc["schema_version"] = serde_json::json!(99),
                    "timestamp" => {
                        doc["started_at_ms"] = serde_json::json!(meeting.sealed_at_ms + 1)
                    }
                    _ => unreachable!(),
                };
                std::fs::write(&file, serde_json::to_vec(&doc).unwrap()).unwrap();
            }
        }
        let (sink, runner) = notification_channel(p.clone(), Arc::new(FakeNotifier::default()));
        let queued = run_asr_one_notifying(&mut store, &Asr(true), &Pi(false), 10000, &sink)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(queued.status, TaskStatus::Queued, "{mode}");
        drop(sink);
        runner.await.unwrap();
        let history = store
            .notification_history(Some(task.session_id), 20)
            .unwrap();
        let record = &history[0];
        assert!(
            matches!(record.meeting_time_source, MeetingTimeSource::Estimated),
            "{mode}"
        );
        assert_ne!(record.meeting_started_at_ms, Some(started), "{mode}");
        assert!(record.identity.contains('约'), "{mode}");
        assert!(!record.identity.contains("PRIVATE_BAD_METADATA"));
        assert_eq!(store.task(task.id).unwrap().status, TaskStatus::Queued);
    }
}

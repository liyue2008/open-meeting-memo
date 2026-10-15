use crate::{
    adapters::{
        asr::LocalAsrAdapter,
        capture::{CapturePaths, MacCaptureAdapter},
        meeting::LarkAdapter,
        notification::MacNotificationAdapter,
        pi::PiAdapter,
    },
    core::{
        config::AppConfig,
        contracts::*,
        lifecycle::{Action, Lifecycle},
        notifications::{
            notification_channel, MeetingIdentity, MeetingTimeSource, NotificationAdapter,
            NotificationEvent, NotificationSink,
        },
        paths::*,
        pipeline,
        process::Reapers,
        runtime::{self, RuntimeGuard, RuntimeLocation},
        store::Store,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};
use uuid::Uuid;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: u32,
    pub command: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub session_id: Option<Uuid>,
    #[serde(default)]
    pub instance_id: Option<Uuid>,
}
#[derive(Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    pub data: Option<Value>,
    pub error: Option<OmmError>,
    #[serde(default)]
    pub service_instance: Option<Uuid>,
}
impl Reply {
    fn result(result: Result<Value>) -> Self {
        match result {
            Ok(data) => Self {
                ok: true,
                data: Some(data),
                error: None,
                service_instance: None,
            },
            Err(error) => Self {
                ok: false,
                data: None,
                error: Some(error),
                service_instance: None,
            },
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Recording {
    pub session_id: Uuid,
    pub title: String,
    pub directory: PathBuf,
    pub source_id: Option<String>,
    pub started_at_ms: i64,
    pub segment_started_ms: i64,
    pub segment_index: u32,
    #[serde(default = "default_segment_ms")]
    pub segment_ms: u64,
    pub config: ConfigSnapshot,
    pub gaps: Vec<String>,
    // Legacy sessions remain protected; only newly persisted unconfirmed intents
    // may be cleared after an authenticated idle observation and an empty directory.
    #[serde(default = "confirmed_legacy_start")]
    pub start_confirmed: bool,
    #[serde(default)]
    pub segments: Vec<AudioSegment>,
    #[serde(default)]
    pub rotation_pending: bool,
    pub sealed: Option<SealedMeeting>,
}
impl Recording {
    fn notification_identity(&self) -> MeetingIdentity {
        MeetingIdentity {
            session_id: Some(self.session_id),
            title: self.title.clone(),
            started_at_ms: Some(self.started_at_ms),
            time_source: MeetingTimeSource::Exact,
            duration_ms: Some(self.segments.iter().fold(0u64, |total, segment| {
                total.saturating_add(segment.duration_ms)
            })),
        }
    }
}
fn confirmed_legacy_start() -> bool {
    true
}
pub struct Controller {
    pub paths: Paths,
    pub capture: Arc<dyn CaptureAdapter>,
    pub instance: Uuid,
    pub lifecycle: Lifecycle,
    pub notifications: NotificationSink,
}
impl Controller {
    pub fn new(
        paths: Paths,
        capture: Arc<dyn CaptureAdapter>,
        instance: Uuid,
        cfg: &AppConfig,
    ) -> Self {
        Self {
            paths,
            capture,
            instance,
            lifecycle: Lifecycle::new(cfg.start_delay_ms, cfg.end_grace_ms),
            notifications: NotificationSink::default(),
        }
    }
    pub fn recording(&self) -> Result<Option<Recording>> {
        Store::open(&self.paths)?.state("recording")
    }
    pub async fn start(&mut self, title: String, source_id: Option<String>) -> Result<Value> {
        let result = self.start_inner(title, source_id).await;
        if let Err(ref error) = result {
            self.recording_failed(error);
        }
        result
    }
    pub fn recording_failed(&self, error: &OmmError) {
        let identity = self
            .recording()
            .ok()
            .flatten()
            .map(|r| r.notification_identity())
            .unwrap_or_default();
        self.notifications.emit_recording_failure(identity, error);
    }
    async fn start_inner(&mut self, title: String, source_id: Option<String>) -> Result<Value> {
        let store = Store::open(&self.paths)?;
        if store.state::<Recording>("recording")?.is_some() {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "recording session already requires reconciliation",
            ));
        }
        if self.capture.status().await?.recording {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "helper recording is not reconciled",
            ));
        }
        let id = Uuid::new_v4();
        let now = chrono::Utc::now().timestamp_millis();
        let dir = self
            .paths
            .data
            .join("meetings")
            .join(chrono::Utc::now().format("%Y-%m-%d").to_string())
            .join(id.to_string());
        private_dir(&dir)?;
        let work = self.paths.cache.join("capture").join(id.to_string());
        private_dir(&work)?;
        let cfg = AppConfig::load(&self.paths)?;
        cfg.validate_policy()?;
        let mut record = Recording {
            session_id: id,
            title,
            directory: dir.clone(),
            source_id: source_id.clone(),
            started_at_ms: now,
            segment_started_ms: now,
            segment_index: 0,
            segment_ms: cfg.segment_ms.min(1_800_000),
            config: cfg.snapshot(),
            gaps: vec![],
            start_confirmed: false,
            segments: vec![],
            rotation_pending: false,
            sealed: None,
        };
        store.set_state("recording", &record)?;
        let request = capture_request(&record, &work);
        let output_path = request.output_path.clone();
        match self.capture.start(request).await {
            Ok(status) => {
                if !status.recording
                    || status.session_id != Some(id)
                    || status.path.as_ref() != Some(&output_path)
                {
                    return Err(OmmError::new(
                        ErrorCode::Conflict,
                        false,
                        "capture start does not match durable recording",
                    ));
                }
                record.start_confirmed = true;
                store.set_state("recording", &record)?;
                self.lifecycle.reconcile_recording(source_id);
                store.clear_state("suppressed_unknown_source")?;
                self.notifications.emit_meeting(
                    NotificationEvent::RecordingStarted,
                    record.notification_identity(),
                );
                Ok(json!(status))
            }
            Err(error) => {
                if let Ok(status) = self.capture.status().await {
                    self.clear_unstarted(&record, &status)?;
                }
                Err(error)
            }
        }
    }
    pub async fn seal(&mut self, manual: bool) -> Result<Value> {
        let result = self.seal_inner(manual).await;
        if let Err(ref error) = result {
            self.recording_failed(error);
        }
        result
    }
    async fn seal_inner(&mut self, manual: bool) -> Result<Value> {
        let mut store = Store::open(&self.paths)?;
        let Some(mut record) = store.state::<Recording>("recording")? else {
            let status = match self.capture.status().await {
                Ok(status) => status,
                Err(error) if error.code == ErrorCode::Unavailable => {
                    return Ok(json!({"alreadyStopped":true,"helperAvailable":false}))
                }
                Err(error) => return Err(error),
            };
            if status.recording {
                return Err(OmmError::new(
                    ErrorCode::Conflict,
                    false,
                    "helper recording missing durable session",
                ));
            }
            return Ok(json!({"recording":false,"alreadyStopped":true}));
        };
        if record.sealed.is_none() {
            let stopped = self.capture.stop().await?;
            if stopped.session_id != Some(record.session_id) {
                if stopped.session_id.is_none()
                    && stopped.segments.is_empty()
                    && self.clear_unstarted(&record, &self.capture.status().await?)?
                {
                    return Ok(json!({"recording":false,"unstartedSessionCleared":true}));
                }
                return Err(OmmError::new(
                    ErrorCode::Conflict,
                    false,
                    "stop does not match durable recording",
                ));
            }
            merge_segments(&mut record.segments, stopped.segments);
            let sealed = SealedMeeting {
                session_id: record.session_id,
                title: record.title.clone(),
                directory: record.directory.clone(),
                segments: record.segments.clone(),
                sealed_at_ms: chrono::Utc::now().timestamp_millis(),
            };
            record.sealed = Some(sealed);
            store.set_state("recording", &record)?;
        }
        let meeting = record.sealed.as_ref().unwrap();
        if meeting.segments.is_empty() {
            store.clear_state("recording")?;
            self.lifecycle.reconcile_recording(None);
            self.notifications.emit_meeting(
                NotificationEvent::RecordingStopped,
                record.notification_identity(),
            );
            return Ok(json!({"meetingId":meeting.session_id,"recording":false,"empty":true}));
        }
        if self.capture.supports_rolling() && !store.has_segmented_session(meeting.session_id)? {
            store.begin_segmented(
                &SealedMeeting {
                    session_id: meeting.session_id,
                    title: meeting.title.clone(),
                    directory: meeting.directory.clone(),
                    segments: vec![],
                    sealed_at_ms: 0,
                },
                &record.config,
            )?;
        }
        let task = if store.has_segmented_session(meeting.session_id)? {
            let latest = AppConfig::load(&self.paths)?;
            latest.validate_policy()?;
            for (sequence, audio) in meeting.segments.iter().enumerate() {
                store.admit_segment_with_audio_config(
                    meeting.session_id,
                    sequence as u32,
                    audio,
                    &latest.audio,
                )?;
            }
            store.seal_segmented(meeting)?
        } else {
            store.enqueue(meeting, &record.config, false)?
        };
        write_private_replace(
            &record.directory.join("meeting.json"),
            &serde_json::to_vec_pretty(&record)?,
        )?;
        store.clear_state("recording")?;
        if manual {
            self.lifecycle.manual_stop();
            store.set_state("suppressed_source", &record.source_id)?;
            store.set_state("suppressed_unknown_source", &record.source_id.is_none())?;
        } else {
            self.lifecycle.reconcile_recording(None);
        }
        self.notifications.emit_meeting(
            NotificationEvent::RecordingStopped,
            record.notification_identity(),
        );
        Ok(json!({"meetingId":meeting.session_id,"task":task,"recording":false}))
    }
    pub fn observation_action(
        &mut self,
        observation: &MeetingObservation,
        consent: bool,
    ) -> Result<Action> {
        let store = Store::open(&self.paths)?;
        if self
            .recording()?
            .is_some_and(|record| record.source_id.is_none())
        {
            return Ok(Action::None);
        }
        let ended =
            observation.state == MeetingState::Inactive && observation.evidence == Evidence::Strong;
        if store
            .state::<bool>("suppressed_unknown_source")?
            .unwrap_or(false)
        {
            if ended {
                store.clear_state("suppressed_unknown_source")?;
            } else {
                return Ok(Action::None);
            }
        }
        if let Some(source) = store
            .state::<Option<String>>("suppressed_source")?
            .flatten()
        {
            if ended {
                store.clear_state("suppressed_source")?;
            } else if observation.source_id.as_ref() == Some(&source) {
                return Ok(Action::None);
            }
        }
        Ok(self.lifecycle.observe(observation, consent))
    }
    pub async fn callback(&mut self, req: &Request) -> Result<Value> {
        if req.instance_id != Some(self.instance)
            || req.session_id != self.recording()?.map(|r| r.session_id)
        {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "capture callback identity mismatch",
            ));
        }
        // Durable acceptance precedes acknowledgement; polling completes stop/queue even after a lost reply.
        Store::open(&self.paths)?.set_state("pending_stop", &req.session_id)?;
        Ok(json!({"accepted":true}))
    }
    pub async fn reconcile(&mut self) -> Result<()> {
        let Some(mut record) = self.recording()? else {
            if self.capture.status().await.is_ok_and(|s| s.recording) {
                return Err(OmmError::new(
                    ErrorCode::Conflict,
                    false,
                    "unowned helper recording needs explicit reconciliation",
                ));
            }
            return Ok(());
        };
        let status = self.capture.status().await?;
        if self.clear_unstarted(&record, &status)? {
            return Ok(());
        }
        if status.session_id != Some(record.session_id) {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "helper and durable session disagree",
            ));
        }
        let mut store = Store::open(&self.paths)?;
        if self.capture.supports_rolling() {
            self.capture.acknowledge_segments(&record.segments)?;
            let closed = self.capture.sealed_segments().await?;
            if !closed.is_empty() && !store.has_segmented_session(record.session_id)? {
                store.begin_segmented(
                    &SealedMeeting {
                        session_id: record.session_id,
                        title: record.title.clone(),
                        directory: record.directory.clone(),
                        segments: vec![],
                        sealed_at_ms: 0,
                    },
                    &record.config,
                )?;
            }
            merge_segments(&mut record.segments, closed.clone());
            if store.has_segmented_session(record.session_id)? {
                let latest = AppConfig::load(&self.paths)?;
                latest.validate_policy()?;
                for (sequence, audio) in record.segments.iter().enumerate() {
                    store.admit_segment_with_audio_config(
                        record.session_id,
                        sequence as u32,
                        audio,
                        &latest.audio,
                    )?;
                }
            }
            store.set_state("recording", &record)?;
            self.capture.acknowledge_segments(&closed)?;
        }
        if !record.start_confirmed {
            record.start_confirmed = true;
            store.set_state("recording", &record)?;
            if status.recording {
                self.notifications.emit_meeting(
                    NotificationEvent::RecordingStarted,
                    record.notification_identity(),
                );
            }
        }
        if record.rotation_pending && !status.recording {
            let stopped = self.capture.stop().await?;
            merge_segments(&mut record.segments, stopped.segments);
            let work = self
                .paths
                .cache
                .join("capture")
                .join(record.session_id.to_string());
            private_dir(&work)?;
            if record
                .directory
                .join(format!("segment-{:04}.wav", record.segment_index))
                .exists()
            {
                record.segment_index += 1;
            }
            store.set_state("recording", &record)?;
            self.capture.start(capture_request(&record, &work)).await?;
            record.rotation_pending = false;
            record.segment_started_ms = chrono::Utc::now().timestamp_millis();
            store.set_state("recording", &record)?;
        } else if !status.recording
            || store.state::<Option<Uuid>>("pending_stop")?.flatten() == Some(record.session_id)
            || record.sealed.is_some()
        {
            self.seal(true).await?;
            store.clear_state("pending_stop")?;
        } else {
            if record.rotation_pending {
                record.rotation_pending = false;
                store.set_state("recording", &record)?;
            }
            self.lifecycle.reconcile_recording(record.source_id);
        }
        Ok(())
    }
    fn clear_unstarted(&mut self, record: &Recording, status: &CaptureStatus) -> Result<bool> {
        // A timed-out start may leave only its durable intent. Never discard an
        // unknown/other session, a partial WAV, or any sign of a previously started session.
        if record.start_confirmed
            || status.recording
            || !status.errors.is_empty()
            || status.session_id.is_some()
            || status.path.is_some()
            || !record.segments.is_empty()
            || record.segment_index != 0
            || record.sealed.is_some()
            || !record.directory.is_absolute()
            || record.directory.canonicalize()? != record.directory
            || !record
                .directory
                .starts_with(self.paths.data.join("meetings"))
            || record.directory.file_name().and_then(|name| name.to_str())
                != Some(record.session_id.to_string().as_str())
            || std::fs::metadata(&record.directory)?.uid() != unsafe { libc::geteuid() }
            || std::fs::metadata(&record.directory)?.mode() & 0o777 != 0o700
            || std::fs::read_dir(&record.directory)?
                .next()
                .transpose()?
                .is_some()
        {
            return Ok(false);
        }
        Store::open(&self.paths)?.clear_state("recording")?;
        self.lifecycle.reconcile_recording(None);
        Ok(true)
    }
    pub async fn rotate(&mut self) -> Result<()> {
        let Some(mut record) = self.recording()? else {
            return Ok(());
        };
        if self.capture.supports_rolling() {
            return self.reconcile().await;
        }
        record.rotation_pending = true;
        Store::open(&self.paths)?.set_state("recording", &record)?;
        let stop = self.capture.stop().await?;
        if stop.session_id != Some(record.session_id) {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "segment rotation identity mismatch",
            ));
        }
        merge_segments(&mut record.segments, stop.segments);
        record.segment_index += 1;
        record.segment_started_ms = chrono::Utc::now().timestamp_millis();
        record.gaps.push("controlled_segment_rotation".into());
        let work = self
            .paths
            .cache
            .join("capture")
            .join(record.session_id.to_string());
        private_dir(&work)?;
        Store::open(&self.paths)?.set_state("recording", &record)?;
        self.capture.start(capture_request(&record, &work)).await?;
        record.rotation_pending = false;
        Store::open(&self.paths)?.set_state("recording", &record)?;
        Ok(())
    }
}
fn merge_segments(existing: &mut Vec<AudioSegment>, segments: Vec<AudioSegment>) {
    for segment in segments {
        if !existing.iter().any(|s| s.path == segment.path) {
            existing.push(segment);
        }
    }
}
fn capture_request(r: &Recording, work: &std::path::Path) -> CaptureRequest {
    CaptureRequest {
        segment_ms: r.segment_ms.min(1_800_000),
        context: TaskContext {
            session_id: r.session_id,
            task_id: r.session_id,
            attempt: r.segment_index + 1,
            lease: Lease {
                token: Uuid::new_v4(),
                expires_at_ms: chrono::Utc::now().timestamp_millis() + 60_000,
            },
            meeting_dir: r.directory.clone(),
            work_dir: work.to_path_buf(),
            config: r.config.clone(),
        },
        output_path: r
            .directory
            .join(format!("segment-{:04}.wav", r.segment_index)),
    }
}
fn check_socket(path: &std::path::Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| {
        OmmError::new(
            ErrorCode::Conflict,
            false,
            "service unreachable; start the user service",
        )
    })?;
    if !meta.file_type().is_socket()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
    {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "service socket not private",
        ));
    }
    Ok(())
}
pub async fn request(paths: &Paths, command: &str, title: Option<String>) -> Result<Value> {
    let location = paths.runtime_location(&std::env::vars().collect())?;
    request_at(paths, &location, command, title).await
}
pub async fn request_at(
    paths: &Paths,
    location: &RuntimeLocation,
    command: &str,
    title: Option<String>,
) -> Result<Value> {
    let socket = location.path.join("service.sock");
    check_socket(&socket)?;
    let owner = runtime::verify_active(&location.path, &paths.state)?;
    let operation = async {
        let mut stream = UnixStream::connect(&socket)
            .await
            .map_err(|_| OmmError::new(ErrorCode::Conflict, false, "service unreachable"))?;
        if stream.peer_cred()?.uid() != owner.uid
            || stream
                .peer_cred()?
                .pid()
                .is_some_and(|pid| pid != owner.pid)
        {
            return Err(OmmError::new(
                ErrorCode::Permission,
                false,
                "service peer does not match runtime owner",
            ));
        }
        stream
            .write_all(&serde_json::to_vec(&Request {
                schema_version: 1,
                command: command.into(),
                title,
                session_id: None,
                instance_id: None,
            })?)
            .await?;
        stream.shutdown().await?;
        let mut bytes = vec![];
        stream.take(1024 * 1024 + 1).read_to_end(&mut bytes).await?;
        if bytes.len() > 1024 * 1024 {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "service response exceeds limit",
            ));
        }
        let reply: Reply = serde_json::from_slice(&bytes)?;
        if reply.service_instance != Some(owner.instance_id) {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "service reply instance does not match runtime owner",
            ));
        }
        if reply.ok {
            reply
                .data
                .ok_or_else(|| OmmError::new(ErrorCode::Corrupt, false, "service data missing"))
        } else {
            Err(reply.error.unwrap_or_else(|| {
                OmmError::new(ErrorCode::Backend, false, "service returned invalid error")
            }))
        }
    };
    tokio::time::timeout(Duration::from_secs(30), operation)
        .await
        .map_err(|_| {
            OmmError::new(
                ErrorCode::Timeout,
                true,
                "service request timed out; reconcile before retry",
            )
        })?
}
pub async fn run(paths: Paths) -> Result<Value> {
    let location = paths.runtime_location(&std::env::vars().collect())?;
    run_at(paths, location).await
}
pub async fn run_at(paths: Paths, location: RuntimeLocation) -> Result<Value> {
    run_at_with_notifier(paths, location, Arc::new(MacNotificationAdapter::default())).await
}
/// Runs the same service loop with an explicit notifier, allowing isolated integration fixtures.
pub async fn run_at_with_notifier(
    paths: Paths,
    location: RuntimeLocation,
    notification_adapter: Arc<dyn NotificationAdapter>,
) -> Result<Value> {
    let cfg = AppConfig::load(&paths)?;
    cfg.validate_policy()?;
    let guard = RuntimeGuard::acquire(&location, &paths.state)?;
    let runtime = &location.path;
    let socket = runtime.join("service.sock");
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let mut store = Store::open(&paths)?;
    let instance = match store.state::<Uuid>("capture_instance")? {
        Some(id) => id,
        None => {
            let id = Uuid::new_v4();
            store.set_state("capture_instance", &id)?;
            id
        }
    };
    for p in [
        paths.state.join("capture"),
        paths.state.join("logs"),
        paths.data.join("meetings"),
        runtime.join("capture"),
    ] {
        private_dir(&p)?;
    }
    let native = Arc::new(MacCaptureAdapter::new(
        CapturePaths {
            state: paths.state.join("capture"),
            logs: paths.state.join("logs"),
            media: paths.data.join("meetings"),
            ipc: runtime.join("capture"),
        },
        instance,
        Duration::from_secs(15),
    )?);
    let (notifications, mut notification_runner) =
        notification_channel(paths.clone(), notification_adapter.clone());
    let mut controller = Controller::new(paths.clone(), native.clone(), instance, &cfg);
    controller.notifications = notifications.clone();
    if let Some(r) = controller.recording()? {
        controller.lifecycle.reconcile_recording(r.source_id);
    }
    store.recover(chrono::Utc::now().timestamp_millis())?;
    store.clean_abandoned_cache(chrono::Utc::now().timestamp_millis())?;
    let pi = Arc::new(PiAdapter::new(paths.clone()));
    store.set_state(
        "pi_cache_recovery",
        &crate::adapters::pi::recover_workspaces(&paths, &store)?,
    )?;
    let asr: Arc<dyn AsrAdapter> = if cfg.worker_executable.is_absolute() {
        Arc::new(LocalAsrAdapter::with_paths(
            cfg.worker_executable.clone(),
            paths.clone(),
        )?)
    } else {
        Arc::new(crate::adapters::UnavailableAsr)
    };
    let worker_asr = asr.clone();
    let codec = Arc::new(crate::adapters::asr::codec::AudioCodec::from_paths(&paths));
    let encoder_codec = codec.clone();
    let encoder_paths = paths.clone();
    let encoder = tokio::spawn(async move {
        let mut store = Store::open(&encoder_paths)?;
        loop {
            if !crate::core::archive_pipeline::run_one(&mut store, encoder_codec.as_ref(), 60_000)
                .await?
            {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), OmmError>(())
    });
    let summary_pi = pi.clone();
    let summary_asr = asr.clone();
    let summary_paths = paths.clone();
    let summary_notifications = notifications.clone();
    let summary_worker = tokio::spawn(async move {
        let mut store = Store::open(&summary_paths)?;
        loop {
            match pipeline::run_summary_one_notifying(
                &mut store,
                summary_asr.as_ref(),
                summary_pi.as_ref(),
                60_000,
                &summary_notifications,
            )
            .await?
            {
                Some(_) => {}
                None => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), OmmError>(())
    });
    let worker_pi = pi.clone();
    let worker_paths = paths.clone();
    let worker_notifications = notifications.clone();
    let worker = tokio::spawn(async move {
        let asr = worker_asr;
        let mut store = Store::open(&worker_paths)?;
        loop {
            match pipeline::run_asr_one_notifying(
                &mut store,
                asr.as_ref(),
                worker_pi.as_ref(),
                60_000,
                &worker_notifications,
            )
            .await?
            {
                Some(_) => {}
                None => tokio::time::sleep(Duration::from_millis(250)).await,
            };
            store.cleanup_archived_sources_for_queue()?;
            store.clean_abandoned_cache(chrono::Utc::now().timestamp_millis())?;
            store.set_state(
                "pi_cache_recovery",
                &crate::adapters::pi::recover_workspaces(&worker_paths, &store)?,
            )?;
        }
        #[allow(unreachable_code)]
        Ok::<(), OmmError>(())
    });
    let observation_reapers = Reapers::default();
    let mut observation_job: Option<ObservationJob> = None;
    let mut control_epoch = 0u64;
    let mut helper = HelperProcess::new(&paths)?;
    helper.observe_endpoint(&native, &paths)?;
    let mut shutdown_reply = None;
    let mut poll = tokio::time::interval(Duration::from_millis(cfg.poll_ms));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_poll = chrono::Utc::now().timestamp_millis();
    let mut terminal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result: Result<Value> = async { loop {
        tokio::select! {
            _=tokio::signal::ctrl_c()=>{break controller.seal(false).await;},
            _=terminal.recv()=>{break controller.seal(false).await;},
            accepted=listener.accept()=> {
                let (mut stream,_)=accepted?;
                if stream.peer_cred()?.uid()!=unsafe{libc::geteuid()}{continue;}
                let input=tokio::time::timeout(Duration::from_secs(10),async{let mut bytes=vec![];(&mut stream).take(1024*1024+1).read_to_end(&mut bytes).await?;if bytes.len()>1024*1024{return Err(OmmError::invalid("service request exceeds limit"));}Ok::<Request,OmmError>(serde_json::from_slice(&bytes)?)}).await;
                let mut reply=match input {
                    Ok(Ok(req)) if req.schema_version==1 => {
                        if matches!(req.command.as_str(), "record_start"|"record_stop"|"capture_auto_stop"|"shutdown") {control_epoch=control_epoch.wrapping_add(1); }
                        helper.observe_endpoint(&native,&paths)?;
                        let action=match req.command.as_str(){
                            "status"=>Ok(json!({"running":true,"paths":paths,"runtime":location,"serviceInstanceId":guard.owner.instance_id,"serviceUid":guard.owner.uid,"servicePid":guard.owner.pid,"singleServiceScope":"selected runtime and persistent STATE; explicitly isolated roots may run independently","capture":native.status().await.ok(),"recording":controller.recording()?.map(|r|json!({"sessionId":r.session_id,"sourceId":r.source_id})),"pipelineHealthy":!worker.is_finished()&&!summary_worker.is_finished()&&!encoder.is_finished(),"asrQueueHealthy":!worker.is_finished(),"summaryQueueHealthy":!summary_worker.is_finished(),"archiveQueueHealthy":!encoder.is_finished(),"piCacheRecovery":Store::open(&paths)?.state::<serde_json::Value>("pi_cache_recovery")?,"lastCaptureError":Store::open(&paths)?.state::<OmmError>("capture_error")?,"lastMeetingObservation":Store::open(&paths)?.state::<serde_json::Value>("meeting_observation")?,"notificationDiagnostic":Store::open(&paths)?.state::<serde_json::Value>("notification_diagnostic")?})),
                            "capture_auto_stop"=>controller.callback(&req).await,
                            "record_status"=>native.status().await.map(|s|json!(s)),
                            "record_start"=>{async {if let Err(e)=ensure_helper(&native,&cfg,&socket,&mut helper,&paths).await {controller.recording_failed(&e);return Err(e);}controller.start(req.title.unwrap_or_else(||"Meeting".into()),None).await}.await},
                            "record_stop"=>controller.seal(true).await,
                            "shutdown"=>{let sealed=controller.seal(false).await;let stopped=match sealed{Ok(value)=>helper.shutdown(&native,&paths).await.map(|()|value),Err(e)=>Err(e)};if stopped.is_ok(){shutdown_reply=Some(stream);break stopped;}stopped},
                            _=>Err(OmmError::invalid("unsupported service command"))};
                        Reply::result(action)
                    },Ok(Err(e))=>Reply::result(Err(e)),_=>Reply::result(Err(OmmError::invalid("invalid or timed out service request"))) };
                reply.service_instance=Some(guard.owner.instance_id);
                let _ = stream.write_all(&serde_json::to_vec(&reply)?).await;let _=stream.shutdown().await;
            },
            _=poll.tick()=> {
                let now=chrono::Utc::now().timestamp_millis();let latest=AppConfig::load(&paths)?;latest.validate_policy()?;
                if now-last_poll>latest.poll_ms as i64*3 {if let Some(mut record)=controller.recording()?{record.gaps.push("sleep_or_service_poll_gap".into());Store::open(&paths)?.set_state("recording",&record)?;}}
                last_poll=now;
                helper.observe_endpoint(&native,&paths)?;
                helper.reap_exited(&native,&paths)?;
                if let Some(mut record)=controller.recording()? {if native.status().await.is_err(){let store=Store::open(&paths)?;if record.sealed.is_none() && store.state::<Option<Uuid>>("pending_stop")?.flatten()!=Some(record.session_id) {record.rotation_pending=true;record.gaps.push("helper_restart".into());store.set_state("recording",&record)?;}
                    if let Err(e)=ensure_helper(&native,&latest,&socket,&mut helper,&paths).await{controller.recording_failed(&e);store.set_state("capture_error",&e)?;continue;}}}
                if let Err(e)=controller.reconcile().await {controller.recording_failed(&e);Store::open(&paths)?.set_state("capture_error",&e)?;}
                if !controller.capture.supports_rolling() && controller.recording()?.is_some_and(|r| now-r.segment_started_ms>=latest.segment_ms.min(1_800_000) as i64){if let Err(e)=controller.rotate().await{controller.recording_failed(&e);Store::open(&paths)?.set_state("capture_error",&e)?;}}
                if latest.auto_record && observation_job.is_none() {
                    let adapter=LarkAdapter::with_reapers(latest.clone(),native.clone(),observation_reapers.clone());
                    observation_job=Some(ObservationJob{epoch:control_epoch,configuration:serde_json::to_vec(&latest)?,task:tokio::spawn(async move{adapter.observe().await})});
                }
            },
            observed=async{(&mut observation_job.as_mut().unwrap().task).await},if observation_job.is_some()=> {
                let job=observation_job.take().unwrap();
                let latest=AppConfig::load(&paths)?;latest.validate_policy()?;
                if job.epoch!=control_epoch || job.configuration!=serde_json::to_vec(&latest)? || !latest.auto_record {continue;}
                let observation=observed.map_err(|_|OmmError::unavailable("meeting observer terminated"))??;
                Store::open(&paths)?.set_state("meeting_observation",&json!({"state":observation.state,"evidence":observation.evidence,"observedAtMs":observation.observed_at_ms,"configurationReady":latest.lark_executable.is_absolute()&&!latest.rtc_log_dirs.is_empty()}))?;
                let action=controller.observation_action(&observation,latest.recording_consent)?;
                if matches!(action,Action::Stop|Action::Switch) {
                    if let Err(e)=controller.seal(false).await{Store::open(&paths)?.set_state("capture_error",&e)?;continue;}
                }
                if matches!(action,Action::Start|Action::Switch) {
                    let start=async {ensure_helper(&native,&latest,&socket,&mut helper,&paths).await?;controller.start(observation.title.unwrap_or_else(||"Lark Meeting".into()),observation.source_id).await}.await;
                    if let Err(e)=start{controller.recording_failed(&e);controller.lifecycle.reconcile_recording(None);Store::open(&paths)?.set_state("capture_error",&e)?;}
                }
            }
        }
    }
    }.await;
    if let Some(job) = observation_job {
        job.task.abort();
        let _ = job.task.await;
    }
    let observation_cleanup = observation_reapers.drain().await;
    worker.abort();
    let _ = worker.await;
    summary_worker.abort();
    let _ = summary_worker.await;
    encoder.abort();
    let _ = encoder.await;
    let asr_cleanup = asr.drain().await;
    let encoder_cleanup = match codec.as_ref() {
        Ok(codec) => codec.drain().await,
        Err(_) => Ok(()),
    };
    let pi_cleanup = pi.drain().await;
    // Only successfully sealed recordings may proceed to helper shutdown. A failed seal
    // retains the original error and the helper/audio for a later reconciliation.
    let capture_cleanup = if result.is_ok() {
        helper.shutdown(&native, &paths).await
    } else {
        Ok(())
    };
    let mut cleanup_errors = Vec::new();
    for cleanup in [
        observation_cleanup,
        asr_cleanup,
        encoder_cleanup,
        pi_cleanup,
        capture_cleanup,
    ] {
        if let Err(error) = cleanup {
            cleanup_errors.push(error);
        }
    }
    let capture_directory = runtime.join("capture");
    match std::fs::read_dir(&capture_directory) {
        Ok(mut entries) => match entries.next() {
            None => {
                if let Err(error) = std::fs::remove_dir(capture_directory) {
                    cleanup_errors.push(error.into());
                }
            }
            Some(Err(error)) => cleanup_errors.push(error.into()),
            Some(Ok(_)) => {}
        },
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            cleanup_errors.push(error.into());
        }
        _ => {}
    }
    if !cleanup_errors.is_empty() {
        if let Err(error) = Store::open(&paths)
            .and_then(|store| store.set_state("shutdown_cleanup_errors", &cleanup_errors))
        {
            cleanup_errors.push(error);
        }
    }
    let result = match result {
        Err(primary) => Err(primary),
        Ok(value) => match cleanup_errors.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(value),
        },
    };
    // Queue cancellation has a budget; confirming owned-child exit may take longer.
    drop(controller);
    drop(notifications);
    if tokio::time::timeout(Duration::from_secs(3), &mut notification_runner)
        .await
        .is_err()
    {
        notification_runner.abort();
        let _ = notification_runner.await;
    }
    if notification_adapter.drain().await.is_err() {
        eprintln!("warning: notification cleanup failed");
    }
    let service_instance = guard.owner.instance_id;
    drop(listener);
    drop(guard);
    // A stop reply is final: no success before helper exit and cancellation cleanup.
    if let Some(mut stream) = shutdown_reply {
        let mut reply = Reply::result(result.clone());
        reply.service_instance = Some(service_instance);
        stream.write_all(&serde_json::to_vec(&reply)?).await?;
        stream.shutdown().await?;
    }
    result
}

/// Durable presence distinguishes a never-started helper from a lost adopted endpoint.
/// Only owned Child exit or the concrete adapter's authenticated exit clears it.
struct HelperProcess {
    child: Option<tokio::process::Child>,
    expected: bool,
}
impl HelperProcess {
    fn new(paths: &Paths) -> Result<Self> {
        Ok(Self {
            child: None,
            expected: Store::open(paths)?
                .state("capture_helper_expected")?
                .unwrap_or(false),
        })
    }
    fn expect(&mut self, paths: &Paths) -> Result<()> {
        self.expected = true;
        Store::open(paths)?.set_state("capture_helper_expected", &true)
    }
    fn clear(&mut self, paths: &Paths) -> Result<()> {
        Store::open(paths)?.clear_state("capture_helper_expected")?;
        self.expected = false;
        Ok(())
    }
    fn endpoint_exists(native: &MacCaptureAdapter) -> Result<bool> {
        match std::fs::symlink_metadata(native.socket_path()) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
    fn observe_endpoint(&mut self, native: &MacCaptureAdapter, paths: &Paths) -> Result<()> {
        if !self.expected && Self::endpoint_exists(native)? {
            self.expect(paths)?;
        }
        Ok(())
    }
    fn reap_exited(&mut self, native: &MacCaptureAdapter, paths: &Paths) -> Result<()> {
        if let Some(child) = &mut self.child {
            if child.try_wait()?.is_some() {
                if !Self::endpoint_exists(native)? {
                    self.clear(paths)?;
                }
                self.child = None;
            }
        }
        Ok(())
    }
    async fn shutdown(&mut self, native: &MacCaptureAdapter, paths: &Paths) -> Result<()> {
        self.observe_endpoint(native, paths)?;
        self.reap_exited(native, paths)?;
        if !self.expected && self.child.is_none() && !Self::endpoint_exists(native)? {
            return Ok(());
        }
        native.shutdown().await?;
        if let Some(child) = &mut self.child {
            tokio::time::timeout(Duration::from_secs(15), child.wait())
                .await
                .map_err(|_| {
                    OmmError::new(ErrorCode::Timeout, true, "capture child exit not confirmed")
                })??;
            self.child = None;
        }
        self.clear(paths)
    }
}

async fn ensure_helper(
    native: &MacCaptureAdapter,
    cfg: &AppConfig,
    socket: &std::path::Path,
    helper: &mut HelperProcess,
    paths: &Paths,
) -> Result<()> {
    // Each status exchange has its own timeout. Bound the entire startup as well,
    // so a helper waiting on desktop permission cannot multiply that by 50 retries.
    tokio::time::timeout(
        Duration::from_secs(15),
        ensure_helper_inner(native, cfg, socket, helper, paths),
    )
    .await
    .map_err(|_| {
        OmmError::new(
            ErrorCode::Timeout,
            true,
            "capture helper startup timed out; check bundle and system permissions",
        )
    })?
}
async fn ensure_helper_inner(
    native: &MacCaptureAdapter,
    cfg: &AppConfig,
    socket: &std::path::Path,
    helper: &mut HelperProcess,
    paths: &Paths,
) -> Result<()> {
    match native.status().await {
        Ok(_) => return helper.expect(paths),
        Err(e) if e.code != ErrorCode::Unavailable => return Err(e),
        _ => {}
    }
    helper.reap_exited(native, paths)?;
    if helper.child.is_none() {
        helper.child = Some(native.launch_helper(&cfg.capture_executable, socket)?);
        helper.expect(paths)?;
    }
    for _ in 0..50 {
        if native.status().await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(OmmError::unavailable(
        "capture helper did not become available; check bundle and system permissions",
    ))
}

struct ObservationJob {
    task: tokio::task::JoinHandle<Result<MeetingObservation>>,
    epoch: u64,
    configuration: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn helper_startup_has_total_deadline_for_unresponsive_endpoint() {
        let temp = tempfile::Builder::new()
            .prefix("omm-start-")
            .tempdir_in("/private/tmp")
            .unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let paths = Paths::resolve(&root.join("h"), &Default::default()).unwrap();
        paths.create_private_dirs().unwrap();
        let capture_paths = CapturePaths {
            state: root.join("s"),
            logs: root.join("l"),
            media: root.join("m"),
            ipc: root.join("r"),
        };
        for directory in [
            &capture_paths.state,
            &capture_paths.logs,
            &capture_paths.media,
            &capture_paths.ipc,
        ] {
            private_dir(directory).unwrap();
        }
        let marker = capture_paths.logs.join("started");
        let executable = root.join("fixture-helper");
        std::fs::write(
            &executable,
            "#!/bin/sh\n/usr/bin/touch \"$YULU_LOG_DIR/started\"\nexec /bin/sleep 60\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let native =
            MacCaptureAdapter::new(capture_paths, Uuid::new_v4(), Duration::from_secs(60)).unwrap();
        let endpoint = native.socket_path();
        let stalled_endpoint = tokio::spawn(async move {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let listener = UnixListener::bind(&endpoint).unwrap();
            std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600)).unwrap();
            let (_stream, _) = listener.accept().await.unwrap();
            // Simulate a permission-blocked helper: accept IPC but never answer.
            std::future::pending::<()>().await;
        });
        let cfg = AppConfig {
            capture_executable: executable,
            ..Default::default()
        };
        let mut helper = HelperProcess::new(&paths).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            ensure_helper(
                &native,
                &cfg,
                &root.join("service.sock"),
                &mut helper,
                &paths,
            ),
        )
        .await;
        stalled_endpoint.abort();
        let _ = stalled_endpoint.await;
        if let Some(mut child) = helper.child.take() {
            child.kill().await.unwrap();
            child.wait().await.unwrap();
        }
        let error = result
            .expect("startup must not accumulate per-exchange timeouts")
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Timeout);
        assert_eq!(
            error.message,
            "capture helper startup timed out; check bundle and system permissions"
        );
    }
}

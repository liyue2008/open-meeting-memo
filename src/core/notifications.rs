//! Notifications contain bounded meeting identifiers and typed reasons, never transcript/error bodies.
use super::{config::AppConfig, contracts::*, paths::Paths, store::Store};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEvent {
    RecordingStarted,
    RecordingStopped,
    SummaryReady,
    RecordingFailed,
    AsrFailed,
    SummaryFailed,
    Test,
}
impl NotificationEvent {
    pub fn message(self) -> &'static str {
        match self {
            Self::RecordingStarted => "录音已开始",
            Self::RecordingStopped => "录音已停止，已排队转写",
            Self::SummaryReady => "会议纪要已生成",
            Self::RecordingFailed => "录音操作失败，请查看服务状态；音频封存状态待确认",
            Self::AsrFailed => "转写失败或被阻塞，请查看任务状态",
            Self::SummaryFailed => "纪要生成失败或被阻塞，请查看任务状态",
            Self::Test => "OMM 测试通知：不含会议内容",
        }
    }
    fn failure(self) -> bool {
        matches!(
            self,
            Self::RecordingFailed | Self::AsrFailed | Self::SummaryFailed
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationConfig {
    pub enabled: bool,
    pub show_meeting_title: bool,
    pub events: Vec<NotificationEvent>,
}
impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            show_meeting_title: true,
            events: vec![
                NotificationEvent::RecordingStarted,
                NotificationEvent::RecordingStopped,
                NotificationEvent::SummaryReady,
                NotificationEvent::RecordingFailed,
                NotificationEvent::AsrFailed,
                NotificationEvent::SummaryFailed,
            ],
        }
    }
}
#[async_trait]
pub trait NotificationAdapter: Send + Sync {
    async fn send(&self, event: NotificationEvent) -> Result<()>;
    async fn send_payload(&self, payload: &NotificationPayload) -> Result<()> {
        self.send(payload.event).await
    }
    async fn drain(&self) -> Result<()> {
        Ok(())
    }
}
#[derive(Clone, Default)]
pub struct NotificationSink {
    sender: Option<mpsc::Sender<Notice>>,
}
struct Notice {
    id: Uuid,
    occurred_at_ms: i64,
    event: NotificationEvent,
    scope: Uuid,
    attempt: u32,
    context: Option<FailureContext>,
    meeting: Option<MeetingIdentity>,
    phase: Option<crate::core::store::StageErrorPhase>,
    retry: Option<(String, u32, Option<i64>)>,
    recording_error: Option<ErrorCode>,
}
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingTimeSource {
    Exact,
    Estimated,
    #[default]
    Unknown,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct MeetingIdentity {
    pub session_id: Option<Uuid>,
    pub title: String,
    pub started_at_ms: Option<i64>,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub time_source: MeetingTimeSource,
}
#[derive(Clone)]
struct FailureContext {
    session_id: Uuid,
    lease: Lease,
    sequence: Option<u32>,
    code: Option<ErrorCode>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct NotificationPayload {
    pub event: NotificationEvent,
    pub identity: String,
    pub safe_reason: Option<ErrorCode>,
    pub retry_status: String,
    pub retry_count: u32,
}
impl NotificationPayload {
    pub fn message(&self) -> String {
        let reason = self.safe_reason.map(safe_reason).unwrap_or("");
        let retry = match self.retry_status.as_str() {
            "queued" => "；重试已排队",
            "exhausted" => "；自动重试次数已用尽",
            "pending" => "；重试待安排",
            "not_retryable" => "；需要处理后再重试",
            _ => "",
        };
        format!(
            "{}：{}{}{}",
            self.identity,
            if self.event == NotificationEvent::RecordingFailed
                && self.identity.contains("会议时间未知")
            {
                "录音启动失败"
            } else {
                self.event.message()
            },
            if reason.is_empty() {
                String::new()
            } else {
                format!("（{reason}）")
            },
            retry
        )
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct NotificationRecord {
    pub schema_version: u32,
    pub id: Uuid,
    pub event: NotificationEvent,
    pub session_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub attempt: u32,
    pub lease: Option<Lease>,
    pub sequence: Option<u32>,
    pub stage: Option<Stage>,
    pub phase: Option<crate::core::store::StageErrorPhase>,
    pub occurred_at_ms: i64,
    pub observed_at_ms: i64,
    pub attempted_at_ms: Option<i64>,
    pub status: String,
    pub visibility: String,
    #[serde(default = "unknown_identity")]
    pub identity: String,
    #[serde(default)]
    pub meeting_started_at_ms: Option<i64>,
    #[serde(default)]
    pub meeting_time_source: MeetingTimeSource,
    #[serde(default)]
    pub first_error_at_ms: Option<i64>,
    pub safe_reason: Option<ErrorCode>,
    pub retry_status: String,
    pub retry_count: u32,
    #[serde(default)]
    pub retry_next_at_ms: Option<i64>,
}
fn unknown_identity() -> String {
    "会议时间未知".into()
}
impl NotificationSink {
    /// Never await a desktop process in capture/control/pipeline operations.
    pub fn emit(&self, event: NotificationEvent, scope: Uuid, attempt: u32) {
        if let Some(sender) = &self.sender {
            if sender
                .try_send(Notice {
                    id: Uuid::new_v4(),
                    occurred_at_ms: chrono::Utc::now().timestamp_millis(),
                    event,
                    scope,
                    attempt,
                    context: None,
                    meeting: None,
                    phase: None,
                    retry: None,
                    recording_error: None,
                })
                .is_err()
            {
                observer_warning("queue_unavailable");
            }
        }
    }
    pub fn emit_failure(
        &self,
        event: NotificationEvent,
        context: &TaskContext,
        sequence: Option<u32>,
        error: &OmmError,
        phase: crate::core::store::StageErrorPhase,
        retry: Option<(String, u32, Option<i64>)>,
    ) {
        self.enqueue(Notice {
            id: Uuid::new_v4(),
            occurred_at_ms: chrono::Utc::now().timestamp_millis(),
            event,
            scope: context.task_id,
            attempt: context.attempt,
            context: Some(FailureContext {
                session_id: context.session_id,
                lease: context.lease.clone(),
                sequence,
                code: Some(error.code),
            }),
            meeting: None,
            phase: Some(phase),
            retry,
            recording_error: None,
        });
    }
    pub fn emit_task(&self, event: NotificationEvent, context: &TaskContext) {
        self.enqueue(Notice {
            id: Uuid::new_v4(),
            occurred_at_ms: chrono::Utc::now().timestamp_millis(),
            event,
            scope: context.task_id,
            attempt: context.attempt,
            context: Some(FailureContext {
                session_id: context.session_id,
                lease: context.lease.clone(),
                sequence: None,
                code: None,
            }),
            meeting: None,
            phase: Some(crate::core::store::StageErrorPhase::Commit),
            retry: None,
            recording_error: None,
        });
    }
    pub fn emit_meeting(&self, event: NotificationEvent, meeting: MeetingIdentity) {
        self.enqueue(Notice {
            id: Uuid::new_v4(),
            occurred_at_ms: chrono::Utc::now().timestamp_millis(),
            event,
            scope: meeting.session_id.unwrap_or_else(Uuid::new_v4),
            attempt: 1,
            context: None,
            meeting: Some(meeting),
            phase: None,
            retry: None,
            recording_error: None,
        });
    }
    pub fn emit_recording_failure(&self, meeting: MeetingIdentity, error: &OmmError) {
        self.enqueue(Notice {
            id: Uuid::new_v4(),
            occurred_at_ms: chrono::Utc::now().timestamp_millis(),
            event: NotificationEvent::RecordingFailed,
            scope: meeting.session_id.unwrap_or_else(Uuid::new_v4),
            attempt: 1,
            context: None,
            meeting: Some(meeting),
            phase: Some(crate::core::store::StageErrorPhase::Adapter),
            retry: None,
            recording_error: Some(error.code),
        });
    }
    fn enqueue(&self, notice: Notice) {
        if let Some(sender) = &self.sender {
            if sender.try_send(notice).is_err() {
                observer_warning("queue_unavailable");
            }
        }
    }
}
fn observer_warning(code: &str) {
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr().lock(),
        "warning: desktop notification {code}"
    );
}
fn diagnostic(paths: &Paths, code: &str) {
    observer_warning(code);
    let _ = Store::open(paths).and_then(|store| store.set_state("notification_diagnostic", &serde_json::json!({"code":code,"atMs":chrono::Utc::now().timestamp_millis(),"visibility":"unknown"})));
}
/// Aborting the executor on early service startup failure prevents a detached notification loop.
pub struct NotificationRunner(tokio::task::JoinHandle<()>);
impl NotificationRunner {
    pub fn abort(&self) {
        self.0.abort();
    }
}
impl std::future::Future for NotificationRunner {
    type Output = std::result::Result<(), tokio::task::JoinError>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::pin::Pin::new(&mut self.0).poll(cx)
    }
}
impl Drop for NotificationRunner {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub trait NotificationClock: Send + Sync {
    /// UTC milliseconds, persisted across service restarts for per-stage failure cooldowns.
    fn now_ms(&self) -> i64;
}
struct SystemNotificationClock;
impl NotificationClock for SystemNotificationClock {
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}
fn safe_reason(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::Timeout => "处理超时",
        ErrorCode::Unavailable => "依赖尚未就绪",
        ErrorCode::Permission | ErrorCode::ConsentRequired | ErrorCode::InteractionRequired => {
            "需要授权或用户操作"
        }
        ErrorCode::InvalidInput | ErrorCode::Corrupt => "音频或处理输入无效",
        ErrorCode::Io => "本地存储或资源错误",
        ErrorCode::Conflict => "状态冲突，需要检查",
        ErrorCode::Backend => "处理服务错误",
    }
}
fn clean_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(60)
        .collect::<String>()
        .trim()
        .to_owned()
}
fn identity_label(meeting: &MeetingIdentity, show_title: bool) -> String {
    let title = clean_title(&meeting.title);
    let generic = matches!(
        title.to_lowercase().replace(' ', "").as_str(),
        "" | "meeting" | "larkmeeting" | "飞书会议" | "会议"
    );
    let date = meeting
        .started_at_ms
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        });
    let name = match date {
        Some(date) => {
            let approximate = if meeting.time_source == MeetingTimeSource::Exact {
                ""
            } else {
                "约"
            };
            if show_title && !generic {
                format!("{title} · {approximate}{date}")
            } else {
                format!("{approximate}{date}开始的会议")
            }
        }
        None if show_title && !generic => format!("{title}（会议时间未知）"),
        None => unknown_identity(),
    };
    if let Some(duration) = meeting.duration_ms.filter(|d| *d > 0) {
        let seconds = duration / 1000;
        format!("{name}（{}分{}秒）", seconds / 60, seconds % 60)
    } else {
        name
    }
}
fn prepare_record(
    store: &Store,
    paths: &Paths,
    notice: &Notice,
    show_title: bool,
    now: i64,
) -> Result<NotificationRecord> {
    let task = if matches!(
        notice.event,
        NotificationEvent::AsrFailed
            | NotificationEvent::SummaryFailed
            | NotificationEvent::SummaryReady
    ) {
        store.task(notice.scope).ok()
    } else {
        None
    };
    let session_id = notice
        .context
        .as_ref()
        .map(|c| c.session_id)
        .or_else(|| task.as_ref().map(|t| t.session_id))
        .or_else(|| notice.meeting.as_ref().and_then(|m| m.session_id));
    let task_id = task
        .as_ref()
        .map(|t| t.id)
        .or_else(|| notice.context.as_ref().map(|_| notice.scope));
    let stage = match notice.event {
        NotificationEvent::AsrFailed => Some(Stage::Transcribe),
        NotificationEvent::SummaryFailed | NotificationEvent::SummaryReady => Some(Stage::Summary),
        _ => None,
    };
    let error = notice
        .context
        .as_ref()
        .and_then(|c| c.code)
        .or(notice.recording_error)
        .or_else(|| task.as_ref().and_then(|t| t.error.as_ref().map(|e| e.code)));
    let lease = notice.context.as_ref().map(|c| c.lease.clone());
    let sequence = notice.context.as_ref().and_then(|c| c.sequence);
    let phase = notice.phase.clone();
    let mut meeting = notice.meeting.clone();
    if let Some(session) = session_id {
        if let Some(ref m) = meeting {
            let _ = store.set_state(&format!("notification-meeting:{session}"), m);
        }
        if !exact_meeting_start(meeting.as_ref()) {
            if let Some(recording) = store.state::<serde_json::Value>("recording")? {
                if recording["session_id"].as_str() == Some(session.to_string().as_str()) {
                    meeting = Some(MeetingIdentity {
                        session_id: Some(session),
                        title: recording["title"].as_str().unwrap_or("").into(),
                        started_at_ms: recording["started_at_ms"].as_i64(),
                        duration_ms: None,
                        time_source: MeetingTimeSource::Exact,
                    });
                }
            }
        }
        let sealed = store.meeting(session)?;
        if !exact_meeting_start(meeting.as_ref()) {
            if let Some(ref sealed) = sealed {
                match persistent_meeting_identity(paths, sealed) {
                    Ok(Some(identity)) => meeting = Some(identity),
                    Ok(None) => {}
                    Err(_) => observer_warning("meeting_metadata_rejected"),
                }
            }
        }
        if !exact_meeting_start(meeting.as_ref()) {
            let cached: Option<MeetingIdentity> =
                store.state(&format!("notification-meeting:{session}"))?;
            if exact_meeting_start(cached.as_ref()) || meeting.is_none() {
                meeting = cached.or(meeting);
            }
        }
        if meeting.as_ref().and_then(|m| m.started_at_ms).is_none() {
            if let Some(m) = sealed {
                let duration = m
                    .segments
                    .iter()
                    .try_fold(0u64, |v, s| v.checked_add(s.duration_ms))
                    .ok_or_else(|| OmmError::invalid("notification duration overflow"))?;
                meeting = Some(MeetingIdentity {
                    session_id: Some(session),
                    title: m.title,
                    started_at_ms: i64::try_from(duration)
                        .ok()
                        .filter(|d| m.sealed_at_ms > 0 && *d <= m.sealed_at_ms)
                        .and_then(|d| m.sealed_at_ms.checked_sub(d)),
                    duration_ms: Some(duration),
                    time_source: MeetingTimeSource::Estimated,
                });
            }
        }
    }
    let meeting_started_at_ms = meeting.as_ref().and_then(|m| m.started_at_ms);
    let meeting_time_source = match meeting.as_ref() {
        Some(m) if m.started_at_ms.is_some() => {
            if m.time_source == MeetingTimeSource::Exact {
                MeetingTimeSource::Exact
            } else {
                MeetingTimeSource::Estimated
            }
        }
        _ => MeetingTimeSource::Unknown,
    };
    let mut identity = meeting
        .map(|m| identity_label(&m, show_title))
        .unwrap_or_else(unknown_identity);
    // Internal correlation IDs never cross the desktop boundary, even in a user-supplied title.
    for id in [session_id, task_id].into_iter().flatten() {
        identity = identity.replace(&id.to_string(), "");
    }
    let (retry_status, retry_count, retry_next_at_ms) = if let Some(retry) = notice.retry.clone() {
        retry
    } else if notice.event.failure() {
        // No failed-claim snapshot means unknown; never splice in a later claim's retry state.
        ("unknown".into(), 0, None)
    } else {
        ("not_applicable".into(), 0, None)
    };
    Ok(NotificationRecord {
        schema_version: 1,
        id: notice.id,
        event: notice.event,
        session_id,
        task_id,
        attempt: notice.attempt,
        lease: lease.clone(),
        sequence,
        stage,
        phase,
        occurred_at_ms: notice.occurred_at_ms,
        observed_at_ms: now,
        attempted_at_ms: None,
        status: "pending".into(),
        visibility: "not_attempted".into(),
        identity,
        meeting_started_at_ms,
        meeting_time_source,
        first_error_at_ms: lease
            .as_ref()
            .and_then(|l| {
                store
                    .state::<crate::core::store::StageAttemptError>(&format!(
                        "stage-error:{}",
                        l.token
                    ))
                    .ok()
                    .flatten()
            })
            .and_then(|e| e.observed_at_ms),
        safe_reason: error,
        retry_status,
        retry_count,
        retry_next_at_ms,
    })
}
fn exact_meeting_start(meeting: Option<&MeetingIdentity>) -> bool {
    meeting.is_some_and(|m| m.time_source == MeetingTimeSource::Exact && m.started_at_ms.is_some())
}

// Read only the existing controller metadata, never transcript/audio or caller-selected paths.
fn persistent_meeting_identity(
    paths: &Paths,
    sealed: &SealedMeeting,
) -> Result<Option<MeetingIdentity>> {
    use super::paths::{controlled_file, validate_private_dir, validate_private_file};
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
        path::PathBuf,
    };
    const METADATA_CAP: u64 = 256 * 1024;
    #[derive(Deserialize)]
    struct SealedMetadata {
        session_id: Uuid,
        directory: PathBuf,
        sealed_at_ms: i64,
    }
    #[derive(Deserialize)]
    struct MeetingMetadata {
        #[serde(default)]
        schema_version: Option<u32>,
        session_id: Uuid,
        directory: PathBuf,
        title: String,
        started_at_ms: i64,
        sealed: SealedMetadata,
    }
    validate_private_dir(&paths.data)?;
    let relative = sealed
        .directory
        .strip_prefix(&paths.data)
        .map_err(|_| OmmError::invalid("meeting directory outside data"))?;
    let mut directory = paths.data.clone();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(OmmError::invalid("meeting directory invalid"));
        }
        directory.push(component);
        validate_private_dir(&directory)?;
    }
    let path = directory.join("meeting.json");
    let before = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    controlled_file(&paths.data, &path)?;
    validate_private_file(&path)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)?;
    let opened = file.metadata()?;
    if !opened.is_file()
        || opened.uid() != unsafe { libc::geteuid() }
        || opened.mode() & 0o777 != 0o600
        || opened.dev() != before.dev()
        || opened.ino() != before.ino()
        || opened.len() > METADATA_CAP
    {
        return Err(OmmError::invalid("meeting metadata unsafe or oversized"));
    }
    let mut bytes = Vec::new();
    file.take(METADATA_CAP + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > METADATA_CAP {
        return Err(OmmError::invalid("meeting metadata oversized"));
    }
    controlled_file(&paths.data, &path)?;
    let after = std::fs::symlink_metadata(&path)?;
    if after.dev() != opened.dev() || after.ino() != opened.ino() {
        return Err(OmmError::invalid("meeting metadata replaced"));
    }
    let metadata: MeetingMetadata = serde_json::from_slice(&bytes)
        .map_err(|_| OmmError::invalid("meeting metadata invalid"))?;
    if metadata.schema_version.is_some_and(|v| v != 1)
        || metadata.session_id != sealed.session_id
        || metadata.sealed.session_id != sealed.session_id
        || metadata.directory != sealed.directory
        || metadata.sealed.directory != sealed.directory
        || metadata.sealed.sealed_at_ms != sealed.sealed_at_ms
        || metadata.started_at_ms <= 0
        || metadata.started_at_ms > sealed.sealed_at_ms
        || chrono::DateTime::from_timestamp_millis(metadata.started_at_ms).is_none()
    {
        return Err(OmmError::invalid(
            "meeting metadata identity or time invalid",
        ));
    }
    let duration = sealed
        .segments
        .iter()
        .try_fold(0u64, |v, s| v.checked_add(s.duration_ms))
        .ok_or_else(|| OmmError::invalid("notification duration overflow"))?;
    Ok(Some(MeetingIdentity {
        session_id: Some(sealed.session_id),
        title: metadata.title,
        started_at_ms: Some(metadata.started_at_ms),
        duration_ms: Some(duration),
        time_source: MeetingTimeSource::Exact,
    }))
}

fn save_record(store: &Store, record: &NotificationRecord) {
    if store.record_notification(record).is_err() {
        observer_warning("history_unavailable");
    }
}
pub fn notification_channel(
    paths: Paths,
    adapter: Arc<dyn NotificationAdapter>,
) -> (NotificationSink, NotificationRunner) {
    notification_channel_with_clock(paths, adapter, Arc::new(SystemNotificationClock))
}
pub fn notification_channel_with_clock(
    paths: Paths,
    adapter: Arc<dyn NotificationAdapter>,
    clock: Arc<dyn NotificationClock>,
) -> (NotificationSink, NotificationRunner) {
    let (sender, mut receiver) = mpsc::channel::<Notice>(16);
    let runner = tokio::spawn(async move {
        while let Some(notice) = receiver.recv().await {
            let preparation = (|| -> Result<_> {
                let cfg = AppConfig::load(&paths)?;
                let store = Store::open(&paths)?;
                let mut record = prepare_record(
                    &store,
                    &paths,
                    &notice,
                    cfg.notifications.show_meeting_title,
                    clock.now_ms(),
                )?;
                let event = serde_json::to_string(&notice.event)?;
                let key = format!("notification:{}:{}:{}", notice.scope, notice.attempt, event);
                let throttle = format!("notification_throttle:{event}");
                record.status = if !cfg.notifications.enabled
                    || !cfg.notifications.events.contains(&notice.event)
                {
                    "disabled"
                } else if !notice.event.failure() && store.state::<bool>(&key)?.unwrap_or(false) {
                    "deduplicated"
                } else if notice.event.failure()
                    && store.state::<i64>(&throttle)?.is_some_and(|last| {
                        record
                            .observed_at_ms
                            .checked_sub(last)
                            .is_some_and(|elapsed| (0..60_000).contains(&elapsed))
                    })
                {
                    "throttled"
                } else {
                    "pending"
                }
                .into();
                save_record(&store, &record);
                Ok((store, record, key, throttle))
            })();
            let (store, mut record, key, throttle) = match preparation {
                Ok(prepared) => prepared,
                Err(_) => {
                    diagnostic(&paths, "configuration_or_state_unavailable");
                    continue;
                }
            };
            if record.status != "pending" {
                continue;
            }
            if notice.event.failure() && store.set_state(&throttle, &record.observed_at_ms).is_err()
            {
                observer_warning("cooldown_unavailable");
            }
            record.attempted_at_ms = Some(clock.now_ms());
            record.visibility = "unknown".into();
            save_record(&store, &record);
            let payload = NotificationPayload {
                event: record.event,
                identity: record.identity.clone(),
                safe_reason: record.safe_reason,
                retry_status: record.retry_status.clone(),
                retry_count: record.retry_count,
            };
            match tokio::time::timeout(Duration::from_secs(3), adapter.send_payload(&payload)).await
            {
                Ok(Ok(())) => {
                    record.status = "sent".into();
                    if !notice.event.failure() && store.set_state(&key, &true).is_err() {
                        observer_warning("receipt_unavailable");
                    }
                }
                Ok(Err(_)) => {
                    record.status = "failed".into();
                    diagnostic(&paths, "call_rejected");
                }
                Err(_) => {
                    record.status = "timed_out".into();
                    diagnostic(&paths, "call_timed_out");
                }
            }
            save_record(&store, &record);
            if adapter.drain().await.is_err() {
                diagnostic(&paths, "cleanup_failed");
            }
        }
    });
    (
        NotificationSink {
            sender: Some(sender),
        },
        NotificationRunner(runner),
    )
}

//! Only generic, typed states cross the notification boundary; meeting content cannot be a payload.
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
    pub events: Vec<NotificationEvent>,
}
impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
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
    async fn drain(&self) -> Result<()> {
        Ok(())
    }
}
#[derive(Clone, Default)]
pub struct NotificationSink {
    sender: Option<mpsc::Sender<Notice>>,
}
struct Notice {
    event: NotificationEvent,
    scope: Uuid,
    attempt: u32,
}
impl NotificationSink {
    /// Never await a desktop process in capture/control/pipeline operations.
    pub fn emit(&self, event: NotificationEvent, scope: Uuid, attempt: u32) {
        if let Some(sender) = &self.sender {
            if sender
                .try_send(Notice {
                    event,
                    scope,
                    attempt,
                })
                .is_err()
            {
                eprintln!("warning: notification queue unavailable or full");
            }
        }
    }
}
fn diagnostic(paths: &Paths, code: &str) {
    eprintln!("warning: desktop notification {code}");
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
            let preparation = (|| -> Result<bool> {
                let cfg = AppConfig::load(&paths)?;
                if !cfg.notifications.enabled || !cfg.notifications.events.contains(&notice.event) {
                    return Ok(false);
                }
                let store = Store::open(&paths)?;
                let event = serde_json::to_string(&notice.event)?;
                let key = format!("notification:{}:{}:{}", notice.scope, notice.attempt, event);
                let deduplicate = !notice.event.failure();
                if deduplicate && store.state::<bool>(&key)?.unwrap_or(false) {
                    return Ok(false);
                }
                let now = clock.now_ms();
                let throttle = format!("notification_throttle:{event}");
                let suppressed = notice.event.failure()
                    && store.state::<i64>(&throttle)?.is_some_and(|last| {
                        now.checked_sub(last)
                            .is_some_and(|elapsed| (0..60_000).contains(&elapsed))
                    });
                if suppressed {
                    return Ok(false);
                }
                // Only successful lifecycle states have permanent receipts. Failed calls
                // consume a cooldown too; skipped failures do not extend it. A wall-clock
                // rollback starts a new cooldown rather than suppressing forever.
                if deduplicate {
                    store.set_state(&key, &true)?;
                }
                if notice.event.failure() {
                    store.set_state(&throttle, &now)?;
                }
                Ok(true)
            })();
            match preparation {
                Ok(false) => continue,
                Err(_) => {
                    diagnostic(&paths, "configuration_or_state_unavailable");
                    continue;
                }
                Ok(true) => {}
            }
            match tokio::time::timeout(Duration::from_secs(3), adapter.send(notice.event)).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => diagnostic(&paths, "call_rejected"),
                Err(_) => diagnostic(&paths, "call_timed_out"),
            }
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

//! First-error evidence is immutable per claim; retries cannot overwrite it.
use super::*;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageErrorPhase {
    Adapter,
    Commit,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StageAttemptError {
    pub schema_version: u32,
    pub task_id: Uuid,
    pub attempt: u32,
    pub lease: Lease,
    pub stage: Stage,
    pub phase: StageErrorPhase,
    pub sequence: Option<u32>,
    pub code: ErrorCode,
    pub retryable: bool,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub observed_at_ms: Option<i64>,
    pub input_hashes: Vec<String>,
}
impl StageAttemptError {
    pub fn new(
        context: &TaskContext,
        stage: Stage,
        phase: StageErrorPhase,
        sequence: Option<u32>,
        input_hashes: Vec<String>,
        elapsed_ms: u64,
        error: &OmmError,
    ) -> Self {
        Self {
            schema_version: 1,
            task_id: context.task_id,
            attempt: context.attempt,
            lease: context.lease.clone(),
            stage,
            phase,
            sequence,
            code: error.code,
            retryable: error.retryable,
            elapsed_ms,
            observed_at_ms: Some(chrono::Utc::now().timestamp_millis()),
            input_hashes,
        }
    }
}
impl Store {
    pub fn stage_progress(&self, task: Uuid) -> Result<Option<serde_json::Value>> {
        self.state(&format!("stage-progress:{task}"))
    }
    pub(super) fn record_legacy_progress(
        &self,
        context: &TaskContext,
        event: &AsrWorkerEvent,
    ) -> Result<()> {
        let (task_id, attempt) = match event {
            AsrWorkerEvent::ChunkProgress {
                task_id, attempt, ..
            }
            | AsrWorkerEvent::Progress {
                task_id, attempt, ..
            } => (*task_id, *attempt),
            _ => return Err(OmmError::invalid("only bounded ASR progress is diagnostic")),
        };
        if task_id != context.task_id || attempt != context.attempt {
            return Err(conflict());
        }
        let now = chrono::Utc::now().timestamp_millis();
        let valid: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE id=?1 AND lease=?2 AND expires>?3 AND stage='transcribe' AND status='transcribing')", params![task_id.to_string(), context.lease.token.to_string(), now], |r| r.get(0))?;
        if !valid {
            return Err(conflict());
        }
        self.set_state(&format!("stage-progress:{task_id}"), &serde_json::json!({"schema_version":1,"lease":context.lease,"phase":"transcribe","observed_at_ms":now,"event":event}))
    }
    pub fn record_stage_error(&self, error: &StageAttemptError) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO settings(key,document) VALUES(?1,?2)",
            params![format!("stage-error:{}", error.lease.token), encode(error)?],
        )?;
        Ok(())
    }
    pub fn stage_error_events(&self, task: Uuid) -> Result<Vec<StageAttemptError>> {
        let mut stmt = self
            .conn
            .prepare("SELECT document FROM settings WHERE key LIKE 'stage-error:%' ORDER BY key")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut events = vec![];
        for row in rows {
            let event: StageAttemptError = decode(&row?)?;
            if event.task_id == task {
                events.push(event);
            }
        }
        Ok(events)
    }
}

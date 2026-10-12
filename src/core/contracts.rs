use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, OmmError>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    Unavailable,
    Permission,
    ConsentRequired,
    InteractionRequired,
    Timeout,
    Conflict,
    Corrupt,
    Io,
    Backend,
}
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct OmmError {
    pub code: ErrorCode,
    pub retryable: bool,
    pub message: String,
}
impl OmmError {
    /// Messages must be sanitized diagnostics: never include child stderr, credentials or meeting text.
    pub fn new(code: ErrorCode, retryable: bool, message: &str) -> Self {
        Self {
            code,
            retryable,
            message: message.into(),
        }
    }
    pub fn unavailable(message: &str) -> Self {
        Self::new(ErrorCode::Unavailable, false, message)
    }
    pub fn invalid(message: &str) -> Self {
        Self::new(ErrorCode::InvalidInput, false, message)
    }
    pub fn exit_code(&self) -> i32 {
        match self.code {
            ErrorCode::InvalidInput => 2,
            ErrorCode::Unavailable
            | ErrorCode::Permission
            | ErrorCode::ConsentRequired
            | ErrorCode::InteractionRequired => 3,
            ErrorCode::Timeout => 5,
            ErrorCode::Conflict => 6,
            _ => 4,
        }
    }
}
impl From<std::io::Error> for OmmError {
    fn from(_: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, true, "local I/O failed")
    }
}
impl From<rusqlite::Error> for OmmError {
    fn from(_: rusqlite::Error) -> Self {
        Self::new(ErrorCode::Io, true, "state database operation failed")
    }
}
impl From<serde_json::Error> for OmmError {
    fn from(_: serde_json::Error) -> Self {
        Self::new(ErrorCode::Corrupt, false, "invalid JSON data")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeetingState {
    Active,
    Inactive,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Strong,
    Weak,
    None,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingObservation {
    pub state: MeetingState,
    pub evidence: Evidence,
    pub observed_at_ms: i64,
    pub source_id: Option<String>,
    pub title: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub app: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSegment {
    pub path: PathBuf,
    pub duration_ms: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedMeeting {
    pub session_id: Uuid,
    pub title: String,
    pub directory: PathBuf,
    pub segments: Vec<AudioSegment>,
    pub sealed_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lease {
    pub token: Uuid,
    pub expires_at_ms: i64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsrConfig {
    pub library_path: PathBuf,
    pub encoder_path: PathBuf,
    pub decoder_path: PathBuf,
    pub tokens_path: PathBuf,
    pub vad_path: PathBuf,
    pub model_version: String,
    pub threads: u32,
    pub timeout_ms: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PiConfig {
    pub executable: PathBuf,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
    pub timeout_ms: u64,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSnapshot {
    pub version: String,
    pub asr: AsrConfig,
    pub pi: PiConfig,
}
impl Default for ConfigSnapshot {
    fn default() -> Self {
        Self {
            version: "1".into(),
            asr: AsrConfig::default(),
            pi: PiConfig::default(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskContext {
    pub session_id: Uuid,
    pub task_id: Uuid,
    pub attempt: u32,
    pub lease: Lease,
    pub meeting_dir: PathBuf,
    pub work_dir: PathBuf,
    pub config: ConfigSnapshot,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureRequest {
    pub context: TaskContext,
    pub output_path: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStatus {
    pub recording: bool,
    pub path: Option<PathBuf>,
    pub ready: bool,
    pub session_id: Option<Uuid>,
    pub errors: Vec<OmmError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureStop {
    pub session_id: Option<Uuid>,
    pub segments: Vec<AudioSegment>,
    pub duration_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrRequest {
    pub context: TaskContext,
    pub segments: Vec<AudioSegment>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioSource {
    Mic,
    System,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSegment {
    pub source: AudioSource,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub schema_version: u32,
    pub model_version: String,
    pub duration_ms: u64,
    pub segments: Vec<TranscriptSegment>,
}
/// Worker stdin carries exactly one JSON request; stdout carries bounded JSONL events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsrWorkerRequest {
    pub schema_version: u32,
    pub request: AsrRequest,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AsrWorkerEvent {
    Progress {
        schema_version: u32,
        task_id: Uuid,
        attempt: u32,
        completed_segments: usize,
        total_segments: usize,
    },
    Result {
        schema_version: u32,
        task_id: Uuid,
        attempt: u32,
        transcript: Transcript,
    },
    Error {
        schema_version: u32,
        task_id: Uuid,
        attempt: u32,
        error: OmmError,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryRequest {
    pub context: TaskContext,
    pub transcript: Transcript,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub schema_version: u32,
    pub markdown: String,
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Transcribe,
    Summary,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Transcribing,
    TranscriptCommitted,
    Summarizing,
    Completed,
    Blocked,
    Failed,
    Interrupted,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryMetadata {
    pub provider: String,
    pub model: String,
    pub prompt_version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSource {
    pub task_id: Uuid,
    pub sha256: String,
    pub model_version: String,
    pub config_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub session_id: Uuid,
    pub attempt: u32,
    pub stage: Stage,
    pub status: TaskStatus,
    pub input_hash: String,
    pub config_hash: String,
    pub summary_metadata: Option<SummaryMetadata>,
    #[serde(default)]
    pub transcript_source: Option<TranscriptSource>,
    pub error: Option<OmmError>,
}
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub task: Task,
    pub stage: Stage,
    pub lease: Lease,
    pub context: TaskContext,
    pub segments: Vec<AudioSegment>,
}

#[async_trait]
pub trait MeetingAdapter: Send + Sync {
    async fn observe(&self) -> Result<MeetingObservation>;
}
#[async_trait]
pub trait CaptureAdapter: Send + Sync {
    async fn start(&self, req: CaptureRequest) -> Result<CaptureStatus>;
    async fn stop(&self) -> Result<CaptureStop>;
    async fn status(&self) -> Result<CaptureStatus>;
    async fn windows(&self) -> Result<Vec<WindowInfo>>;
    async fn devices(&self) -> Result<Vec<AudioDevice>>;
}
#[async_trait]
pub trait AsrAdapter: Send + Sync {
    async fn transcribe(&self, req: AsrRequest) -> Result<Transcript>;
}
#[async_trait]
pub trait SummaryAdapter: Send + Sync {
    async fn summarize(&self, req: SummaryRequest) -> Result<Summary>;
}

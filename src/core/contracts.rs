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
pub struct CaptureSegmentIdentity {
    pub session_id: Uuid,
    pub instance_id: Uuid,
    pub sequence: u32,
    pub offset_frames: u64,
    pub frame_count: u64,
    pub sample_rate: u32,
    pub descriptor_sha256: String,
    #[serde(default)]
    pub legacy_wav: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSegment {
    pub path: PathBuf,
    pub duration_ms: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<CaptureSegmentIdentity>,
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
    /// Explicit runtime for JavaScript launchers; never inherited from the user's PATH.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_executable: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureSourceClock {
    pub rate_hz: f64,
    pub input_frames: u64,
    pub target_frames: u64,
    pub rate_changes: u32,
    pub accepted_channels: u32,
    pub input_elapsed_us: u64,
    pub first_accepted_offset_us: u64,
    pub last_accepted_offset_us: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureClock {
    pub schema_version: u32,
    pub target_rate_hz: u32,
    pub writer_started_utc_ms: u64,
    pub elapsed_us: u64,
    pub format_rejects: u64,
    pub mic: Option<CaptureSourceClock>,
    pub system: Option<CaptureSourceClock>,
}
impl CaptureClock {
    pub fn valid(&self) -> bool {
        self.schema_version == 1
            && self.target_rate_hz == 48000
            && self.writer_started_utc_ms <= i64::MAX as u64
            && self.elapsed_us <= i64::MAX as u64
            && (self.writer_started_utc_ms > 0
                || (self.elapsed_us == 0 && self.mic.is_none() && self.system.is_none()))
            && [(&self.mic, 1), (&self.system, 2)]
                .into_iter()
                .all(|(source, channels)| {
                    source.as_ref().is_none_or(|s| {
                        s.rate_hz.is_finite()
                            && (8000.0..=384000.0).contains(&s.rate_hz)
                            && s.accepted_channels == channels
                            && s.input_frames <= i64::MAX as u64
                            && s.target_frames <= i64::MAX as u64
                            && s.first_accepted_offset_us <= s.last_accepted_offset_us
                            && s.last_accepted_offset_us <= self.elapsed_us
                            && s.input_elapsed_us
                                == s.last_accepted_offset_us - s.first_accepted_offset_us
                    })
                })
    }
}
/// Missing clocks are legacy; a present null, unknown schema or malformed clock is not legacy.
pub fn deserialize_capture_clock<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<CaptureClock>, D::Error> {
    let clock = CaptureClock::deserialize(d)?;
    if !clock.valid() {
        return Err(serde::de::Error::custom("invalid capture clock"));
    }
    Ok(Some(clock))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioConfig {
    pub opus_bitrate_kbps: u32,
}
impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            opus_bitrate_kbps: 48,
        }
    }
}
impl AudioConfig {
    pub fn validate(&self) -> Result<()> {
        if ![32, 48, 64, 96].contains(&self.opus_bitrate_kbps) {
            return Err(OmmError::invalid(
                "Opus bitrate must be 32, 48, 64 or 96 kbps",
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSnapshot {
    #[serde(default)]
    pub audio: AudioConfig,
    pub version: String,
    pub asr: AsrConfig,
    pub pi: PiConfig,
}
impl Default for ConfigSnapshot {
    fn default() -> Self {
        Self {
            audio: AudioConfig::default(),
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
    #[serde(default = "default_segment_ms")]
    pub segment_ms: u64,
}
pub fn default_segment_ms() -> u64 {
    1_800_000
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
    #[serde(default)]
    pub source_kind: Option<AudioSource>,
    pub context: TaskContext,
    pub segments: Vec<AudioSegment>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AudioSource {
    Mix,
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
    #[serde(default)]
    pub chunk_checkpoint_dir: Option<PathBuf>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsrStartupTiming {
    pub validate_audio_ms: u64,
    pub validate_models_ms: u64,
    pub load_model_ms: u64,
    pub prepare_audio_ms: u64,
}
impl AsrStartupTiming {
    pub fn valid(&self) -> bool {
        [
            self.validate_audio_ms,
            self.validate_models_ms,
            self.load_model_ms,
            self.prepare_audio_ms,
        ]
        .into_iter()
        .all(|v| v <= 86_400_000)
    }
}
fn deserialize_startup_timing<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<AsrStartupTiming>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<AsrStartupTiming>(value)
        .ok()
        .filter(AsrStartupTiming::valid))
}
/// Numeric observer data only. Durations use monotonic clocks; CPU may exceed wall on multiple threads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsrChunkTiming {
    pub phase: AsrChunkPhase,
    pub input_index: u32,
    pub window_index: u32,
    pub core_start_sample: u64,
    pub decode_start_sample: u64,
    pub core_samples: u32,
    pub decode_samples: u32,
    pub cache_hit: bool,
    pub cache_lookup_us: u64,
    pub native_decode_wall_us: u64,
    pub process_cpu_us: Option<u64>,
    pub checkpoint_write_us: u64,
    pub token_count: Option<u32>,
    pub output_chars: u32,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AsrChunkPhase {
    CacheHit,
    NativeDecodeTotal,
}
impl AsrChunkTiming {
    pub fn valid(&self) -> bool {
        let max_us = 86_400_000_000u64;
        self.input_index < 4096
            && self.window_index < 1_000_000
            && self.core_start_sample <= i32::MAX as u64
            && self.core_samples > 0
            && self.core_samples <= 393600
            && self.decode_start_sample <= self.core_start_sample
            && self.core_start_sample - self.decode_start_sample <= 3200
            && self.decode_samples as u64
                >= self.core_samples as u64 + self.core_start_sample - self.decode_start_sample
            && self.decode_samples as u64
                - (self.core_samples as u64 + self.core_start_sample - self.decode_start_sample)
                <= 3200
            && self.phase
                == if self.cache_hit {
                    AsrChunkPhase::CacheHit
                } else {
                    AsrChunkPhase::NativeDecodeTotal
                }
            && self.decode_samples >= self.core_samples
            && self.decode_samples <= 400000
            && self.cache_lookup_us <= max_us
            && self.native_decode_wall_us <= max_us
            && self.checkpoint_write_us <= max_us
            && self.process_cpu_us.is_none_or(|v| {
                v <= self
                    .native_decode_wall_us
                    .saturating_mul(1024)
                    .saturating_add(100000)
            })
            && self.output_chars <= 1024 * 1024
            && self.token_count.is_none_or(|v| v <= 1_000_000)
            && (!self.cache_hit
                || (self.native_decode_wall_us == 0
                    && self.process_cpu_us == Some(0)
                    && self.checkpoint_write_us == 0
                    && self.token_count.is_none()))
            && serde_json::to_vec(self).is_ok_and(|b| b.len() <= 1024)
    }
}
fn deserialize_chunk_timing<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<AsrChunkTiming>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value::<AsrChunkTiming>(value)
        .ok()
        .filter(AsrChunkTiming::valid))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AsrWorkerEvent {
    ChunkProgress {
        schema_version: u32,
        task_id: Uuid,
        attempt: u32,
        source: AudioSource,
        completed_chunks: usize,
        vad_spans: usize,
        speech_samples: u64,
        processing_ms: u64,
        #[serde(
            default,
            deserialize_with = "deserialize_startup_timing",
            skip_serializing_if = "Option::is_none"
        )]
        startup_timing: Option<AsrStartupTiming>,
        #[serde(
            default,
            deserialize_with = "deserialize_chunk_timing",
            skip_serializing_if = "Option::is_none"
        )]
        chunk_timing: Option<AsrChunkTiming>,
    },
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
    fn supports_rolling(&self) -> bool {
        false
    }
    async fn sealed_segments(&self) -> Result<Vec<AudioSegment>> {
        Ok(vec![])
    }
    /// Called only after descriptors and their jobs are durably saved.
    fn acknowledge_segments(&self, _segments: &[AudioSegment]) -> Result<()> {
        Ok(())
    }
}
#[async_trait]
pub trait AsrAdapter: Send + Sync {
    async fn transcribe(&self, req: AsrRequest) -> Result<Transcript>;
    async fn drain(&self) -> Result<()> {
        Ok(())
    }
}
#[async_trait]
pub trait SummaryAdapter: Send + Sync {
    async fn drain(&self) -> Result<()> {
        Ok(())
    }
    async fn summarize(&self, req: SummaryRequest) -> Result<Summary>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpusFile {
    pub pre_skip: u16,
    pub tail_padding: u64,
    pub decoded_frames: u64,
    pub path: PathBuf,
    pub sha256: String,
    pub source: String,
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u16,
    pub container: String,
    pub codec: String,
    pub bitrate: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveReceipt {
    /// None on old receipts means unknown, never inferred from current user configuration.
    #[serde(default)]
    pub encoder_version: Option<String>,
    pub schema_version: u32,
    pub source_sha256: String,
    pub files: Vec<OpusFile>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedAudio {
    pub sequence: u32,
    pub offset_ms: u64,
    pub file: OpusFile,
}

/// Rolling audio uses one frame clock; pre-rolling WAV callers retain their millisecond sums.
pub fn audio_ranges_ms(audio: &[AudioSegment]) -> Result<Vec<(u64, u64)>> {
    let rolling = audio.iter().any(|a| a.capture.is_some());
    let mut ranges = Vec::with_capacity(audio.len());
    let mut end = 0u64;
    let first = audio.first().and_then(|a| a.capture.as_ref());
    let epoch = first.map(|c| c.offset_frames).unwrap_or(0);
    let mut previous_frames = epoch;
    for (i, a) in audio.iter().enumerate() {
        let range = if rolling {
            let c = a
                .capture
                .as_ref()
                .ok_or_else(|| OmmError::invalid("mixed rolling and legacy audio clocks"))?;
            let first = first.ok_or_else(|| OmmError::invalid("rolling clock has no origin"))?;
            if c.sample_rate != 48000
                || c.frame_count == 0
                || c.offset_frames != previous_frames
                || c.session_id != first.session_id
                || c.instance_id != first.instance_id
                || first.sequence.checked_add(
                    u32::try_from(i).map_err(|_| OmmError::invalid("segment count overflow"))?,
                ) != Some(c.sequence)
                || c.frame_count.checked_mul(1000).map(|f| f / 48000) != Some(a.duration_ms)
            {
                return Err(OmmError::invalid(
                    "rolling frame clock differs from sealed input",
                ));
            }
            previous_frames = c
                .offset_frames
                .checked_add(c.frame_count)
                .ok_or_else(|| OmmError::invalid("audio frame overflow"))?;
            let start = (c.offset_frames - epoch)
                .checked_mul(1000)
                .ok_or_else(|| OmmError::invalid("audio duration overflow"))?
                / 48000;
            let end = (previous_frames - epoch)
                .checked_mul(1000)
                .ok_or_else(|| OmmError::invalid("audio duration overflow"))?
                / 48000;
            (start, end)
        } else {
            (
                end,
                end.checked_add(a.duration_ms)
                    .ok_or_else(|| OmmError::invalid("audio duration overflow"))?,
            )
        };
        end = range.1;
        ranges.push(range);
    }
    Ok(ranges)
}

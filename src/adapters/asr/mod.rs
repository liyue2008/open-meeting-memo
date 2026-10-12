//! Local-only ASR adapter. All C++/ONNX execution occurs in a separate worker.
pub mod audio;
mod ffi;
pub mod models;
mod native;

use crate::core::contracts::*;
use async_trait::async_trait;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

const REQUEST_LIMIT: usize = 1024 * 1024;
const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;
const LINE_LIMIT: usize = 16 * 1024 * 1024;

pub struct LocalAsrAdapter {
    worker: PathBuf,
}
struct AttemptDirectory(PathBuf);
impl AttemptDirectory {
    fn new(work: &Path) -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let path = work.join(format!("asr-worker-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}
impl Drop for AttemptDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
impl LocalAsrAdapter {
    /// Core supplies the trusted absolute executable; no PATH/cwd discovery.
    pub fn new(worker: PathBuf) -> Result<Self> {
        if !worker.is_absolute() {
            return Err(OmmError::invalid("ASR worker executable must be absolute"));
        }
        Ok(Self { worker })
    }
}
#[async_trait]
impl AsrAdapter for LocalAsrAdapter {
    async fn transcribe(&self, req: AsrRequest) -> Result<Transcript> {
        validate_request(&req)?;
        if req.context.config.asr.timeout_ms == 0 {
            return Err(OmmError::invalid("ASR worker timeout must be positive"));
        }
        let timeout = req.context.config.asr.timeout_ms;
        // Parent owns disposable cache lifetime even if the native worker aborts.
        let attempt = AttemptDirectory::new(&req.context.work_dir)?;
        let mut worker_request = req.clone();
        worker_request.context.work_dir = attempt.0.clone();
        let input = serde_json::to_vec(&AsrWorkerRequest {
            schema_version: 1,
            request: worker_request,
        })?;
        if input.len() > REQUEST_LIMIT {
            return Err(OmmError::invalid("ASR request exceeds protocol limit"));
        }
        let mut child = Command::new(&self.worker)
            .args(["--protocol", "1"])
            .current_dir(&attempt.0)
            .env_clear()
            .env("OMP_NUM_THREADS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| OmmError::unavailable("configured ASR worker could not start"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| OmmError::unavailable("ASR worker stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OmmError::unavailable("ASR worker stdout unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| OmmError::unavailable("ASR worker stderr unavailable"))?;
        let operation = async {
            let send = async {
                stdin.write_all(&input).await?;
                stdin.shutdown().await?;
                drop(stdin);
                Ok::<(), OmmError>(())
            };
            let ((), output, _) = tokio::try_join!(
                send,
                bounded_read(stdout, OUTPUT_LIMIT),
                bounded_read(stderr, 64 * 1024)
            )?;
            let status = child.wait().await?;
            let parsed = parse_events(&output, &req);
            if !status.success() {
                // Only protocol-defined sanitized errors are retained, never raw child stderr.
                let terminal_error = output
                    .split(|b| *b == b'\n')
                    .rfind(|l| !l.is_empty())
                    .is_some_and(|line| {
                        matches!(
                            serde_json::from_slice::<AsrWorkerEvent>(line),
                            Ok(AsrWorkerEvent::Error { .. })
                        )
                    });
                return match parsed {
                    Err(e) if terminal_error => Err(e),
                    _ => Err(OmmError::new(
                        ErrorCode::Backend,
                        true,
                        "ASR worker exited unsuccessfully or crashed; diagnostics withheld",
                    )),
                };
            }
            parsed
        };
        match tokio::time::timeout(Duration::from_millis(timeout), operation).await {
            Ok(result) => {
                if result.is_err() {
                    let _ = child.kill().await;
                }
                result
            }
            Err(_) => {
                let _ = child.kill().await;
                Err(OmmError::new(
                    ErrorCode::Timeout,
                    true,
                    "local ASR worker timed out",
                ))
            }
        }
    }
}
async fn bounded_read(mut reader: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![];
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        if bytes.len().saturating_add(n) > limit {
            return Err(resource_limit(
                "ASR worker output exceeded protocol resource limit",
            ));
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    Ok(bytes)
}
fn parse_events(bytes: &[u8], req: &AsrRequest) -> Result<Transcript> {
    let mut result = None;
    let mut error = None;
    let mut previous = 0;
    for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        if line.len() > LINE_LIMIT {
            return Err(resource_limit(
                "ASR worker event exceeds protocol line resource limit",
            ));
        }
        if result.is_some() || error.is_some() {
            return Err(audio::corrupt("invalid ASR worker event ordering"));
        }
        let event: AsrWorkerEvent = serde_json::from_slice(line)?;
        let (schema, task, attempt) = match &event {
            AsrWorkerEvent::Progress {
                schema_version,
                task_id,
                attempt,
                ..
            }
            | AsrWorkerEvent::Result {
                schema_version,
                task_id,
                attempt,
                ..
            }
            | AsrWorkerEvent::Error {
                schema_version,
                task_id,
                attempt,
                ..
            } => (*schema_version, *task_id, *attempt),
        };
        if schema != 1 || task != req.context.task_id || attempt != req.context.attempt {
            return Err(audio::corrupt("ASR worker event identity mismatch"));
        }
        match event {
            AsrWorkerEvent::Progress {
                completed_segments,
                total_segments,
                ..
            } => {
                if total_segments != req.segments.len()
                    || completed_segments < previous
                    || completed_segments > total_segments
                {
                    return Err(audio::corrupt("invalid ASR worker progress"));
                }
                previous = completed_segments;
            }
            AsrWorkerEvent::Result { transcript, .. } => {
                validate_transcript(&transcript, req)?;
                result = Some(transcript);
            }
            AsrWorkerEvent::Error { error: e, .. } => {
                // Native library stderr/messages may contain paths/text: retain code only.
                error = Some(OmmError::new(e.code, e.retryable, match e.code {
                    ErrorCode::Unavailable => "ASR library or pinned model unavailable; verify explicit local installation",
                    ErrorCode::Corrupt => "ASR input/model hash or protocol validation failed",
                    ErrorCode::InvalidInput => "ASR request or pinned configuration invalid",
                    ErrorCode::Backend => "local native ASR failed or exceeded protocol/resource limits",
                    _ => "local ASR worker failed; diagnostic contents withheld",
                }));
            }
        }
    }
    if let Some(e) = error {
        return Err(e);
    }
    result.ok_or_else(|| audio::corrupt("ASR worker returned no result"))
}
fn canonical(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(OmmError::invalid("ASR paths must be absolute"));
    }
    let resolved = path.canonicalize()?;
    if resolved != path {
        return Err(OmmError::invalid(
            "ASR paths must be canonical without symlinks",
        ));
    }
    Ok(resolved)
}
fn validate_request(req: &AsrRequest) -> Result<()> {
    let root = canonical(&req.context.meeting_dir)?;
    let work = canonical(&req.context.work_dir)?;
    if !root.is_dir() || !work.is_dir() || req.segments.is_empty() {
        return Err(OmmError::invalid(
            "ASR requires controlled directories and sealed segments",
        ));
    }
    if req.context.lease.expires_at_ms <= chrono::Utc::now().timestamp_millis() {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "ASR task lease expired",
        ));
    }
    let mut total = 0u64;
    for segment in &req.segments {
        if !canonical(&segment.path)?.starts_with(&root)
            || !segment.path.is_file()
            || segment.duration_ms == 0
        {
            return Err(OmmError::invalid(
                "sealed ASR audio must be a file inside the meeting directory",
            ));
        }
        total = total
            .checked_add(segment.duration_ms)
            .ok_or_else(|| OmmError::invalid("sealed duration overflow"))?;
        if segment.sha256.len() != 64 || models::sha256(&segment.path)? != segment.sha256 {
            return Err(audio::corrupt("sealed audio SHA256 mismatch"));
        }
    }
    Ok(())
}
fn validate_transcript(t: &Transcript, req: &AsrRequest) -> Result<()> {
    if t.schema_version != 1
        || t.model_version != req.context.config.asr.model_version
        || t.duration_ms != req.segments.iter().map(|s| s.duration_ms).sum::<u64>()
    {
        return Err(audio::corrupt("ASR transcript metadata mismatch"));
    }
    let mut prev = 0;
    for s in &t.segments {
        if s.start_ms < prev
            || s.start_ms >= s.end_ms
            || s.end_ms > t.duration_ms
            || s.text.trim().is_empty()
        {
            return Err(audio::corrupt("ASR transcript bounds or text invalid"));
        }
        prev = s.start_ms;
    }
    Ok(())
}
fn resource_limit(message: &str) -> OmmError {
    OmmError::new(ErrorCode::Backend, false, message)
}
fn emit(event: &AsrWorkerEvent) -> Result<()> {
    let bytes = serde_json::to_vec(event)?;
    if bytes.len() > LINE_LIMIT {
        return Err(resource_limit(
            "ASR worker result exceeds protocol line resource limit",
        ));
    }
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&bytes)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}
/// Synchronous process entrypoint called only by core's required-features worker binary.
pub fn worker_main() -> Result<()> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--protocol", "1"] {
        return Err(OmmError::invalid("ASR worker requires --protocol 1"));
    }
    let mut bytes = vec![];
    std::io::stdin()
        .take((REQUEST_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > REQUEST_LIMIT {
        return Err(OmmError::invalid("ASR worker request exceeds limit"));
    }
    let input: AsrWorkerRequest = serde_json::from_slice(&bytes)?;
    let req = input.request;
    let result = if input.schema_version == 1 {
        transcribe_local(&req)
    } else {
        Err(OmmError::invalid("unsupported ASR worker schema"))
    };
    let result = result.and_then(|transcript| {
        emit(&AsrWorkerEvent::Result {
            schema_version: 1,
            task_id: req.context.task_id,
            attempt: req.context.attempt,
            transcript,
        })
    });
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            emit(&AsrWorkerEvent::Error {
                schema_version: 1,
                task_id: req.context.task_id,
                attempt: req.context.attempt,
                error: error.clone(),
            })?;
            Err(error)
        }
    }
}
fn transcribe_local(req: &AsrRequest) -> Result<Transcript> {
    validate_request(req)?;
    let cfg = &req.context.config.asr;
    if !cfg.library_path.is_absolute() || !cfg.library_path.is_file() {
        return Err(OmmError::unavailable(
            "explicit absolute sherpa-onnx library file is required",
        ));
    }
    models::verify_models(cfg)?;
    let engine = native::NativeEngine::new(cfg)?;
    let mut offset = 0;
    let mut segments = vec![];
    let mut output_bytes = 0;
    for (i, sealed) in req.segments.iter().enumerate() {
        let prepared =
            audio::prepare_sources(&sealed.path, sealed.duration_ms, &req.context.work_dir)?;
        for (channel, path) in prepared.paths.iter().enumerate() {
            let source = if channel == 0 {
                AudioSource::Mic
            } else {
                AudioSource::System
            };
            let mut part = engine.transcribe_file(path, prepared.samples, source, offset)?;
            // Rounded VAD milliseconds may exceed the sealed duration by one tick.
            for segment in &mut part {
                segment.end_ms = segment.end_ms.min(offset + sealed.duration_ms);
            }
            output_bytes += serde_json::to_vec(&part)?.len();
            if output_bytes > LINE_LIMIT - 1024 {
                return Err(resource_limit(
                    "ASR transcript exceeds protocol line resource limit",
                ));
            }
            segments.extend(part);
        }
        offset += sealed.duration_ms;
        emit(&AsrWorkerEvent::Progress {
            schema_version: 1,
            task_id: req.context.task_id,
            attempt: req.context.attempt,
            completed_segments: i + 1,
            total_segments: req.segments.len(),
        })?;
    }
    let transcript = Transcript {
        schema_version: 1,
        model_version: cfg.model_version.clone(),
        duration_ms: offset,
        segments: audio::deduplicate(segments),
    };
    validate_transcript(&transcript, req)?;
    Ok(transcript)
}

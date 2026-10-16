//! Local-only ASR adapter. All C++/ONNX execution occurs in a separate worker.
pub mod audio;
mod chunks;
pub mod codec;
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
    paths: Option<crate::core::paths::Paths>,
    reapers: crate::core::process::Reapers,
}
struct AttemptDirectory(PathBuf);
struct AttemptResources {
    _attempt: AttemptDirectory,
    _decoded: Option<AttemptDirectory>,
    _reader: Option<std::fs::File>,
}
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
        Ok(Self {
            worker,
            paths: None,
            reapers: Default::default(),
        })
    }
    pub fn with_paths(worker: PathBuf, paths: crate::core::paths::Paths) -> Result<Self> {
        let mut adapter = Self::new(worker)?;
        adapter.paths = Some(paths);
        Ok(adapter)
    }
    pub async fn drain(&self) -> Result<()> {
        self.reapers.drain().await
    }
}
#[async_trait]
impl AsrAdapter for LocalAsrAdapter {
    async fn drain(&self) -> Result<()> {
        self.reapers.drain().await
    }
    async fn transcribe(&self, req: AsrRequest) -> Result<Transcript> {
        if req.context.config.asr.timeout_ms == 0 {
            return Err(OmmError::invalid("ASR worker timeout must be positive"));
        }
        let deadline = tokio::time::Instant::now().checked_add(Duration::from_millis(req.context.config.asr.timeout_ms))
            .ok_or_else(|| OmmError::invalid("ASR deadline overflow"))?;
        let reader = match &self.paths {
            Some(paths) => Some(crate::core::store::Store::audio_read_lock_until(paths, deadline).await?),
            None => None,
        };
        // Maintenance may have removed temporary WAVs while we waited. Select inputs only under the read lock.
        let mut prepared = req.clone();
        let mut codec_lineage = vec![None; req.segments.len()];
        let mut decoded = None;
        if req.segments.iter().any(|s| !s.path.exists()) {
            let paths = self.paths.as_ref().ok_or_else(|| {
                OmmError::unavailable("archived audio requires locked codec and state")
            })?;
            let codec = codec::AudioCodec::from_paths(paths)?.with_reapers(self.reapers.clone());
            let store = crate::core::store::Store::open_read_only(paths)?;
            let directory = AttemptDirectory::new(&req.context.meeting_dir)?;
            let mut decode_context = req.context.clone();
            decode_context.work_dir = directory.0.clone();
            for (index, audio) in prepared.segments.iter_mut().enumerate() {
                if !audio.path.exists() {
                    let receipt = store
                        .archive_for_audio(req.context.session_id, audio)?
                        .ok_or_else(|| OmmError::unavailable("audio archive receipt missing"))?;
                    codec_lineage[index] = Some(receipt.clone());
                    let duration = audio.duration_ms;
                    let capture = audio.capture.clone();
                    let output = tokio::time::timeout_at(deadline, codec.decode_sources(&receipt, &decode_context)).await;
                    *audio = match output {
                        Ok(result) => result?,
                        Err(_) => {
                            codec.drain().await?;
                            return Err(OmmError::new(ErrorCode::Timeout, true, "ASR deadline expired decoding audio"));
                        }
                    };
                    audio.duration_ms = duration;
                    audio.capture = capture;
                }
            }
            codec.drain().await?;
            decoded = Some(directory);
        }
        validate_request(&prepared)?;
        if let Some(paths) = &self.paths {
            let store = crate::core::store::Store::open_read_only(paths)?;
            let source_archives = req.segments.iter().map(|audio| store.archive_for_audio(req.context.session_id, audio)).collect::<Result<Vec<_>>>()?;
            let inputs: Vec<_> = req.segments.iter().zip(&prepared.segments).zip(&codec_lineage).zip(&source_archives).map(|(((original,actual),lineage),source_archive)| serde_json::json!({
                "original_sha256":original.sha256,"actual_sha256":actual.sha256,
                "capture":original.capture,"codec_lineage":lineage,"source_archive":source_archive,"input_container":"wav","duration_ms":actual.duration_ms
            })).collect();
            crate::core::store::Store::open(paths)?.record_asr_inputs(&req.context,&serde_json::json!({
                "schema_version":1,"checkpoint_version":1,"requested_audio":req.context.config.audio,
                "config_hash":crate::core::store::hash_bytes(&serde_json::to_vec(&req.context.config.asr)?),"inputs":inputs
            }))?;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(OmmError::new(ErrorCode::Timeout, true, "ASR deadline expired preparing audio"));
        }
        // Parent owns disposable cache lifetime even if the native worker aborts.
        let attempt = AttemptDirectory::new(&req.context.work_dir)?;
        let mut worker_request = prepared;
        worker_request.context.work_dir = attempt.0.clone();
        let input = serde_json::to_vec(&AsrWorkerRequest {
            schema_version: 1,
            request: worker_request,
            chunk_checkpoint_dir: req.context.work_dir.parent().map(|p| p.join(".chunks")),
        })?;
        if input.len() > REQUEST_LIMIT {
            return Err(OmmError::invalid("ASR request exceeds protocol limit"));
        }
        let mut command = Command::new(&self.worker);
        command
            .args(["--protocol", "1"])
            .current_dir(&attempt.0)
            .env_clear()
            .env("OMP_NUM_THREADS", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(lock) = reader.as_ref() {
            crate::core::process::inherit_audio_reader(&mut command, lock);
        }
        let child = command
            .spawn()
            .map_err(|_| OmmError::unavailable("configured ASR worker could not start"))?;
        let mut child = crate::core::process::OwnedChild::new(
            child,
            self.reapers.clone(),
            Some(std::sync::Arc::new(AttemptResources {
                _attempt: attempt,
                _decoded: decoded,
                _reader: reader,
            })),
        );
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
                event_read(stdout, &req, self.paths.as_ref()),
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
        match tokio::time::timeout_at(deadline, operation).await {
            Ok(result) => {
                if result.is_err() {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                }
                result
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(OmmError::new(
                    ErrorCode::Timeout,
                    true,
                    "local ASR worker timed out",
                ))
            }
        }
    }
}
fn parse_worker_event(line: &[u8]) -> Result<AsrWorkerEvent> {
    match serde_json::from_slice(line) {
        Ok(event) => Ok(event),
        Err(error) => {
            // Internal-tag buffering rejects numbers such as 1e999 before the optional field visitor.
            // Retry without diagnostics, preserving strict validation of every business field.
            let Ok(fields) = serde_json::from_slice::<BusinessEvent<'_>>(line) else {return Err(error.into());};
            if !fields.had_observer {return Err(error.into());}
            Ok(serde_json::from_slice(&serde_json::to_vec(&fields)?)?)
        }
    }
}
// Preserve raw nested business values and top-level order; never let optional diagnostics
// turn duplicate business keys into last-write-wins protocol data.
struct BusinessEvent<'a> {
    fields: Vec<(String, &'a serde_json::value::RawValue)>,
    had_observer: bool,
}
impl<'de> serde::Deserialize<'de> for BusinessEvent<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct EventVisitor;
        impl<'de> serde::de::Visitor<'de> for EventVisitor {
            type Value = BusinessEvent<'de>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an ASR event object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> std::result::Result<Self::Value, M::Error> {
                let mut event = BusinessEvent { fields: vec![], had_observer: false };
                let mut seen = std::collections::BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    let value = map.next_value::<&'de serde_json::value::RawValue>()?;
                    if matches!(key.as_str(), "chunk_timing" | "startup_timing") {
                        event.had_observer = true;
                    } else {
                        if !seen.insert(key.clone()) {
                            return Err(serde::de::Error::custom("duplicate business field"));
                        }
                        event.fields.push((key, value));
                    }
                }
                Ok(event)
            }
        }
        deserializer.deserialize_map(EventVisitor)
    }
}
impl serde::Serialize for BusinessEvent<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.fields.len()))?;
        for (key, value) in &self.fields { map.serialize_entry(key, value)?; }
        map.end()
    }
}
async fn event_read(
    mut reader: impl AsyncRead + Unpin,
    req: &AsrRequest,
    paths: Option<&crate::core::paths::Paths>,
) -> Result<Vec<u8>> {
    let mut output = vec![];
    let mut line = vec![];
    let mut chunk = [0u8; 8192];
    let mut terminal = false;
    let mut previous = 0;
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        for &byte in &chunk[..n] {
            if byte == b'\n' {
                if line.is_empty() {
                    continue;
                }
                if terminal {
                    return Err(audio::corrupt("invalid ASR worker event ordering"));
                }
                let mut event = parse_worker_event(&line)?;
                if let AsrWorkerEvent::ChunkProgress{chunk_timing,..}=&mut event {
                    if chunk_timing.as_ref().is_some_and(|t|t.input_index as usize>=req.segments.len()) {*chunk_timing=None;}
                }
                // Progress is validated/persisted online, not accumulated until a long worker exits.
                if matches!(event,AsrWorkerEvent::Result{..}|AsrWorkerEvent::Error{..}) {
                    let terminal_line=serde_json::to_vec(&event)?;
                    if output.len().saturating_add(terminal_line.len()+1)>OUTPUT_LIMIT{return Err(resource_limit("ASR worker output exceeded protocol resource limit"));}
                    output.extend_from_slice(&terminal_line);output.push(b'\n');
                }
                line.clear();
                let (schema, task, attempt) = match &event {
                    AsrWorkerEvent::Progress {
                        schema_version,
                        task_id,
                        attempt,
                        ..
                    }
                    | AsrWorkerEvent::ChunkProgress {
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
                match &event {
                    AsrWorkerEvent::Progress {
                        completed_segments,
                        total_segments,
                        ..
                    } => {
                        if *total_segments != req.segments.len()
                            || *completed_segments < previous
                            || completed_segments > total_segments
                        {
                            return Err(audio::corrupt("invalid ASR worker progress"));
                        }
                        previous = *completed_segments;
                    }
                    AsrWorkerEvent::ChunkProgress {
                        completed_chunks,
                        vad_spans,
                        ..
                    } => {
                        if completed_chunks > vad_spans {
                            return Err(audio::corrupt("invalid ASR chunk progress"));
                        }
                    }
                    _ => terminal = true,
                }
                if matches!(
                    event,
                    AsrWorkerEvent::ChunkProgress { .. } | AsrWorkerEvent::Progress { .. }
                ) {
                    if let Some(paths) = paths {
                        if let Err(error) = crate::core::store::Store::open(paths)
                            .and_then(|store| {
                                store.record_asr_measurement_best_effort(&req.context,&event);
                                store.record_segment_progress(&req.context, &event)
                            })
                        {
                            use std::io::Write;
                            let _ = writeln!(std::io::stderr().lock(), "warning: ASR progress unavailable code={:?}", error.code);
                        }
                    }
                }
            } else {
                line.push(byte);
                if line.len() > LINE_LIMIT {
                    return Err(resource_limit(
                        "ASR worker event exceeds protocol line resource limit",
                    ));
                }
            }
        }
    }
    if !line.is_empty() {
        return Err(audio::corrupt("ASR worker event missing newline"));
    }
    Ok(output)
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
            AsrWorkerEvent::ChunkProgress {
                schema_version,
                task_id,
                attempt,
                ..
            }
            | AsrWorkerEvent::Progress {
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
            AsrWorkerEvent::ChunkProgress {
                completed_chunks,
                vad_spans,
                ..
            } => {
                if completed_chunks > vad_spans {
                    return Err(audio::corrupt("invalid ASR chunk progress"));
                }
            }
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
    audio_ranges_ms(&req.segments)?;
    for segment in &req.segments {
        if !canonical(&segment.path)?.starts_with(&root)
            || !segment.path.is_file()
            || segment.duration_ms == 0
        {
            return Err(OmmError::invalid(
                "sealed ASR audio must be a file inside the meeting directory",
            ));
        }
        if let Some(capture) = &segment.capture {
            let reader = hound::WavReader::open(&segment.path).map_err(|_| audio::corrupt("rolling input is not a WAV"))?;
            if capture.session_id != req.context.session_id || u64::from(reader.duration()) != capture.frame_count || reader.spec().sample_rate != capture.sample_rate {
                return Err(audio::corrupt("rolling input frames differ from capture clock"));
            }
        }
        if segment.sha256.len() != 64 || models::sha256(&segment.path)? != segment.sha256 {
            return Err(audio::corrupt("sealed audio SHA256 mismatch"));
        }
    }
    Ok(())
}
fn validate_transcript(t: &Transcript, req: &AsrRequest) -> Result<()> {
    if t.schema_version != 1
        || t.model_version != req.context.config.asr.model_version
        || t.duration_ms != audio_ranges_ms(&req.segments)?.last().map(|r| r.1).unwrap_or(0)
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
        transcribe_local(&req, input.chunk_checkpoint_dir.as_deref())
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
fn transcribe_local(req: &AsrRequest, checkpoint: Option<&Path>) -> Result<Transcript> {
    let start = std::time::Instant::now();
    validate_request(req)?;
    let validate_audio_ms = start.elapsed().as_millis() as u64;
    if let Some(root) = checkpoint {
        let expected = req
            .context
            .work_dir
            .parent()
            .and_then(Path::parent)
            .map(|p| p.join(".chunks"));
        if expected.as_deref() != Some(root) {
            return Err(OmmError::invalid(
                "chunk checkpoint root differs from owned workspace",
            ));
        }
        crate::core::paths::private_dir(root)?;
    }
    let cfg = &req.context.config.asr;
    if !cfg.library_path.is_absolute() || !cfg.library_path.is_file() {
        return Err(OmmError::unavailable(
            "explicit absolute sherpa-onnx library file is required",
        ));
    }
    let start = std::time::Instant::now();
    models::verify_models(cfg)?;
    let validate_models_ms = start.elapsed().as_millis() as u64;
    let start = std::time::Instant::now();
    let engine = native::NativeEngine::new(cfg)?;
    let load_model_ms = start.elapsed().as_millis() as u64;
    let ranges = audio_ranges_ms(&req.segments)?;
    let mut segments = vec![];
    let mut output_bytes = 0;
    for (i, sealed) in req.segments.iter().enumerate() {
        let (offset, sealed_end) = ranges[i];
        let start = std::time::Instant::now();
        let prepared =
            audio::prepare_sources(&sealed.path, sealed.duration_ms, &req.context.work_dir)?;
        let startup_timing = Some(AsrStartupTiming { validate_audio_ms, validate_models_ms, load_model_ms,
            prepare_audio_ms: start.elapsed().as_millis() as u64 });
        for (channel, path) in prepared.paths.iter().enumerate() {
            let source = if channel == 0 {
                AudioSource::Mic
            } else {
                AudioSource::System
            };
            let (task_id, attempt) = (req.context.task_id, req.context.attempt);
            let cache = checkpoint
                .map(|root| chunks::directory_with_source(root, path, cfg, source, &sealed.sha256))
                .transpose()?;
            let mut part = engine.transcribe_file_measured(
                path,
                prepared.samples,
                source,
                offset,
                cache,
                Box::new(
                    move |completed_chunks, vad_spans, speech_samples, processing_ms, chunk_timing| {
                        let chunk_timing=chunk_timing.map(|mut t|{t.input_index=i as u32;t});
                        emit(&AsrWorkerEvent::ChunkProgress {
                            schema_version: 1,
                            task_id,
                            attempt,
                            source,
                            completed_chunks,
                            vad_spans,
                            speech_samples,
                            processing_ms,
                            startup_timing,
                            chunk_timing,
                        })
                    },
                ),
            )?;
            // Rounded VAD milliseconds may exceed the sealed duration by one tick.
            for segment in &mut part {
                segment.end_ms = segment.end_ms.min(sealed_end);
            }
            output_bytes += serde_json::to_vec(&part)?.len();
            if output_bytes > LINE_LIMIT - 1024 {
                return Err(resource_limit(
                    "ASR transcript exceeds protocol line resource limit",
                ));
            }
            segments.extend(part);
        }
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
        duration_ms: ranges.last().map(|r| r.1).unwrap_or(0),
        segments: audio::deduplicate(segments),
    };
    validate_transcript(&transcript, req)?;
    Ok(transcript)
}

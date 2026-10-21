//! EOF-framed Yulu capture perimeter. Construction/status never launch capture or request TCC.
use crate::core::{
    contracts::*,
    paths::{controlled_file, validate_private_dir},
};
use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek},
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    process::{Child, Command},
    sync::Mutex,
};
use uuid::Uuid;

const MAX_RESPONSE: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct CapturePaths {
    /// STATE/omm/capture: minimal config projection and helper recovery state only.
    pub state: PathBuf,
    pub logs: PathBuf,
    pub media: PathBuf,
    /// Current login runtime capture directory, already validated by the core provider.
    pub ipc: PathBuf,
}
impl CapturePaths {
    fn validate(&self) -> Result<()> {
        for p in [&self.state, &self.logs, &self.media, &self.ipc] {
            canonical_private(p)?;
        }
        for p in [&self.state, &self.logs, &self.ipc] {
            if p.starts_with(&self.media) || self.media.starts_with(p) {
                return Err(OmmError::invalid(
                    "capture media overlaps operational directory",
                ));
            }
        }
        Ok(())
    }
}
fn canonical_private(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.canonicalize()? != path {
        return Err(OmmError::invalid(
            "capture directory must be canonical and absolute",
        ));
    }
    validate_private_dir(path)
}
fn validate_socket_path(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() >= 104 {
        return Err(OmmError::invalid(
            "capture socket path must be absolute and shorter than 104 bytes",
        ));
    }
    canonical_private(
        path.parent()
            .ok_or_else(|| OmmError::invalid("socket has no parent"))?,
    )
}
fn conflict() -> OmmError {
    OmmError::new(
        ErrorCode::Conflict,
        false,
        "capture instance or session requires reconciliation",
    )
}
fn backend_error(value: &Value) -> OmmError {
    match value.get("error").and_then(Value::as_str).unwrap_or("") {
        "capture_instance_conflict"
        | "capture_protocol_conflict"
        | "capture_shutting_down"
        | "already_recording"
        | "capture_session_conflict"
        | "application_transaction_in_progress" | "permissions_busy" => conflict(),
        "unsafe_output_dir" | "invalid_capture_request" | "application_transaction_lock_unsafe" => {
            OmmError::new(
                ErrorCode::Permission,
                false,
                "capture rejected unsafe request",
            )
        }
        "capture_recovery_state_failed" => OmmError::new(
            ErrorCode::Io,
            true,
            "capture recovery state could not be saved",
        ),
        "sys_capture_not_ready" | "mic_capture_not_ready" => OmmError::new(
            ErrorCode::Permission,
            false,
            "capture source unavailable; check system permissions and devices",
        ),
        _ => OmmError::new(ErrorCode::Backend, true, "capture helper rejected request"),
    }
}

/// The owner is a persistent helper-instance UUID, not a fresh service-restart UUID.
/// Core must status/reconcile first after restart; this adapter refuses duplicate start.
pub struct MacCaptureAdapter {
    paths: CapturePaths,
    instance_id: Uuid,
    timeout: Duration,
    operation: Mutex<()>,
    sealed_seen: std::sync::Mutex<std::collections::BTreeMap<PathBuf, AudioSegment>>,
    shutdown_confirmed: std::sync::atomic::AtomicBool,
}
impl MacCaptureAdapter {
    pub fn new(paths: CapturePaths, instance_id: Uuid, timeout: Duration) -> Result<Self> {
        paths.validate()?;
        validate_socket_path(&paths.ipc.join("audio_daemon.sock"))?;
        if timeout.is_zero() {
            return Err(OmmError::invalid("capture timeout must be positive"));
        }
        Ok(Self {
            paths,
            instance_id,
            timeout,
            operation: Mutex::new(()),
            sealed_seen: Default::default(),
            shutdown_confirmed: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub fn socket_path(&self) -> PathBuf {
        self.paths.ipc.join("audio_daemon.sock")
    }

    /// Explicit lifecycle action only. Core owns child monitoring and reaping.
    /// Launch may resume an interrupted recording; never call it as a status probe.
    pub fn launch_helper(&self, executable: &Path, service_socket: &Path) -> Result<Child> {
        self.launch_helper_mode(executable, service_socket, false)
    }
    /// Permission preparation cannot resume an interrupted recording.
    pub fn launch_helper_for_permissions(&self, executable: &Path, service_socket: &Path) -> Result<Child> {
        self.launch_helper_mode(executable, service_socket, true)
    }
    fn launch_helper_mode(&self, executable: &Path, service_socket: &Path, permissions_only: bool) -> Result<Child> {
        self.paths.validate()?;
        validate_socket_path(service_socket)?;
        if !executable.is_absolute()
            || executable.canonicalize()? != executable
            || !std::fs::metadata(executable)?.is_file()
        {
            return Err(OmmError::invalid(
                "capture executable must be a trusted absolute bundle path",
            ));
        }
        let mut command = Command::new(executable);
        if permissions_only { command.arg("--permissions-only"); }
        command
            .env_clear()
            .env("YULU_APPLICATION_SUPPORT_DIR", &self.paths.state)
            .env("YULU_LOG_DIR", &self.paths.logs)
            .env("YULU_MEDIA_LIBRARY_DIR", &self.paths.media)
            .env("YULU_IPC_DIR", &self.paths.ipc)
            .env("YULU_SERVICE_OWNER", self.instance_id.to_string())
            .env("OMM_SERVICE_SOCKET", service_socket)
            .current_dir(
                executable
                    .parent()
                    .ok_or_else(|| OmmError::invalid("missing bundle directory"))?,
            )
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
            .spawn()
            .map_err(|_| OmmError::unavailable("capture helper could not be launched"))
    }

    /// Explicit user operation only; never interprets capture readiness as TCC.
    pub async fn request_permissions(&self, request_id: Uuid) -> Result<CapturePermissions> {
        let _guard = self.operation.lock().await;
        let socket = self.socket_path();
        let identity = SocketIdentity::read(&socket)?.ok_or_else(|| OmmError::unavailable("capture helper unavailable"))?;
        let operation = async {
            let mut stream = shutdown_connect(&socket, identity).await?;
            let value = exchange_connected(&mut stream, &serde_json::to_vec(&json!({
                "action":"permissions_request", "instance_id":self.instance_id,
                "request_id":request_id, "captureIPCVersion":1
            }))?).await?;
            self.instance_matches(&value)?;
            let reply: CapturePermissions = serde_json::from_value(value)?;
            if reply.request_id != request_id || reply.system_audio != "unknown"
                || !["authorized", "denied", "restricted", "not_determined", "unknown"].contains(&reply.microphone.as_str())
                || !["completed", "unconfirmed", "timed_out", "cancelled"].contains(&reply.outcome.as_str()) {
                return Err(conflict());
            }
            Ok(reply)
        };
        tokio::time::timeout(self.timeout, operation).await.map_err(|_| OmmError::new(ErrorCode::Timeout, false, "permission request unconfirmed; the OS dialog may remain pending"))?
    }
    pub async fn cancel_permissions(&self, request_id: Uuid) -> Result<Value> {
        let operation = async {
            let identity = SocketIdentity::read(&self.socket_path())?.ok_or_else(conflict)?;
            let mut stream = shutdown_connect(&self.socket_path(), identity).await?;
            let value = exchange_connected(&mut stream, &serde_json::to_vec(&json!({"action":"permissions_cancel", "instance_id":self.instance_id, "request_id":request_id, "captureIPCVersion":1}))?).await?;
            self.instance_matches(&value)?;
            if value["request_id"] != request_id.to_string() || value["outcome"] != "cancelled" { return Err(conflict()); }
            Ok(value)
        };
        tokio::time::timeout(Duration::from_secs(4), operation).await.map_err(|_| OmmError::new(ErrorCode::Timeout,false,"permission cancellation unconfirmed; OS dialog may remain pending"))?
    }

    /// Quit only an idle, authenticated instance and wait for its kernel process-exit event.
    /// Success is cached only for this adapter with the endpoint still absent. A fresh
    /// adapter cannot prove exit from a missing socket and returns unavailable instead.
    pub async fn shutdown(&self) -> Result<()> {
        let operation = async {
            let _guard = self.operation.lock().await;
            self.paths.validate()?;
            let socket = self.socket_path();
            validate_socket_path(&socket)?;
            if self
                .shutdown_confirmed
                .load(std::sync::atomic::Ordering::Acquire)
            {
                return if SocketIdentity::read(&socket)?.is_none() {
                    Ok(())
                } else {
                    Err(conflict())
                };
            }
            #[cfg(target_os = "macos")]
            {
                self.shutdown_inner(&socket).await?;
                self.shutdown_confirmed
                    .store(true, std::sync::atomic::Ordering::Release);
                Ok(())
            }
            #[cfg(not(target_os = "macos"))]
            Err(OmmError::unavailable(
                "capture process-exit verification requires macOS",
            ))
        };
        tokio::time::timeout(self.timeout, operation)
            .await
            .map_err(|_| {
                OmmError::new(
                    ErrorCode::Timeout,
                    true,
                    "capture shutdown timed out; exit not confirmed",
                )
            })?
    }
    #[cfg(target_os = "macos")]
    async fn shutdown_inner(&self, socket: &Path) -> Result<()> {
        let identity = SocketIdentity::read(socket)?.ok_or_else(|| {
            OmmError::unavailable("capture endpoint absent; exit cannot be confirmed")
        })?;
        let mut status_stream = shutdown_connect(socket, identity).await?;
        let pid = shutdown_process::peer_pid(&status_stream)?;
        // Register while connected, BEFORE requesting status/quit: NOTE_EXIT pins this
        // process in the kernel, unlike polling kill(pid, 0), which risks PID reuse.
        let watcher = shutdown_process::ExitWatcher::new(pid)?;
        let status = exchange_connected(&mut status_stream, br#"{"action":"status"}"#).await?;
        self.decode_status(status.clone())?;
        if status.get("pid").and_then(Value::as_i64) != Some(i64::from(pid)) {
            return Err(conflict());
        }
        if status.get("recording").and_then(Value::as_bool) == Some(true) {
            return Err(conflict());
        }
        drop(status_stream);
        let mut quit_stream = shutdown_connect(socket, identity).await?;
        if shutdown_process::peer_pid(&quit_stream)? != pid {
            return Err(conflict());
        }
        let ack = exchange_connected(
            &mut quit_stream,
            &serde_json::to_vec(&json!({
                "action":"quit", "instance_id":self.instance_id, "captureIPCVersion":1
            }))?,
        )
        .await?;
        self.instance_matches(&ack)?;
        if ack.get("captureIPCVersion").and_then(Value::as_u64) != Some(1)
            || ack.get("pid").and_then(Value::as_i64) != Some(i64::from(pid))
        {
            return Err(conflict());
        }
        if ack.get("status").and_then(Value::as_str) != Some("bye") {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "capture quit acknowledgement invalid",
            ));
        }
        while !watcher.exited()? {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Do not remove anything ourselves. A new endpoint is a conflict, not proof
        // of this instance's shutdown; a stale owned endpoint is a cleanup failure.
        match SocketIdentity::read(socket)? {
            None => Ok(()),
            Some(current) if current != identity => Err(conflict()),
            Some(_) => Err(OmmError::new(
                ErrorCode::Io,
                true,
                "capture exited without cleaning its endpoint",
            )),
        }
    }
    async fn exchange(&self, request: Value) -> Result<Value> {
        let socket = self.socket_path();
        validate_socket_path(&socket)?;
        let metadata = std::fs::symlink_metadata(&socket)
            .map_err(|_| OmmError::unavailable("capture helper socket unavailable"))?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(OmmError::new(
                ErrorCode::Permission,
                false,
                "capture socket is not private",
            ));
        }
        let bytes = serde_json::to_vec(&request)?;
        let operation = async {
            let mut stream = UnixStream::connect(socket)
                .await
                .map_err(|_| OmmError::unavailable("capture helper unreachable"))?;
            exchange_connected(&mut stream, &bytes).await
        };
        tokio::time::timeout(self.timeout, operation)
            .await
            .map_err(|_| {
                OmmError::new(
                    ErrorCode::Timeout,
                    true,
                    "capture helper response timed out; reconcile before retry",
                )
            })?
    }
    fn instance_matches(&self, value: &Value) -> Result<()> {
        if value.get("instance_id").and_then(Value::as_str)
            != Some(self.instance_id.to_string().as_str())
        {
            return Err(conflict());
        }
        Ok(())
    }
    fn decode_status(&self, value: Value) -> Result<CaptureStatus> {
        self.instance_matches(&value)?;
        if value.get("captureIPCVersion").and_then(Value::as_u64) != Some(1) {
            return Err(conflict());
        }
        let recording = value
            .get("recording")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "capture status missing recording state",
                )
            })?;
        let path = value
            .get("file")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        if let Some(p) = &path {
            controlled_file(&self.paths.media, p)?;
        }
        let session_id = value
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| conflict())?;
        if recording && (path.is_none() || session_id.is_none()) {
            return Err(conflict());
        }
        let ready = value.get("sysReady").and_then(Value::as_bool) == Some(true)
            && value.get("micReady").and_then(Value::as_bool) == Some(true);
        let mut errors = vec![];
        for key in ["sysError", "micError", "bridgeError"] {
            if value
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
            {
                errors.push(OmmError::new(
                    if key == "bridgeError" {
                        ErrorCode::Io
                    } else {
                        ErrorCode::Backend
                    },
                    true,
                    "capture source or recovery state reported an error",
                ));
            }
        }
        Ok(CaptureStatus {
            recording,
            path,
            ready,
            session_id,
            errors,
        })
    }
    async fn status_inner(&self) -> Result<CaptureStatus> {
        self.decode_status(self.exchange(json!({"action": "status"})).await?)
    }
    fn sealed_cursor(&self) -> Result<Option<u32>> {
        let seen = self.sealed_seen.lock().map_err(|_| conflict())?;
        Ok(seen.values().filter_map(|a|a.capture.as_ref().map(|c|c.sequence)).max())
    }
    async fn all_sealed(&self, session: Uuid) -> Result<Vec<AudioSegment>> {
        // Final seal replays the immutable manifest; live admission uses the acknowledged cursor.
        let mut result: Vec<AudioSegment> = vec![];
        let mut cursor = None;
        loop {
            let (page,more)=self.sealed_page(session,cursor).await?;
            if page.is_empty() && more { return Err(conflict()); }
            if let Some(last)=page.last() { cursor=last.capture.as_ref().map(|c|c.sequence); }
            result.extend(page);
            if result.len()>4096 {return Err(OmmError::invalid("too many capture segments"));}
            if !more {return Ok(result);}
        }
    }
    async fn sealed_page(&self, session: Uuid, cursor: Option<u32>) -> Result<(Vec<AudioSegment>,bool)> {
        let value=self.exchange(json!({"action":"sealed_segments","instance_id":self.instance_id,"session_id":session,"after_sequence":cursor.map(i64::from).unwrap_or(-1),"limit":8})).await?;
        self.instance_matches(&value)?;
        if value.get("session_id").and_then(Value::as_str).and_then(|s|Uuid::parse_str(s).ok())!=Some(session) || serde_json::to_vec(&value)?.len()>131072 {return Err(conflict());}
        let origin:Uuid=serde_json::from_value(value.get("origin_instance_id").cloned().ok_or_else(conflict)?)?;
        let page=value.get("segments").and_then(Value::as_array).ok_or_else(conflict)?;
        if page.len()>8 {return Err(conflict());}
        let mut result=vec![];let mut expected=cursor.map(|c|c.checked_add(1).ok_or_else(conflict)).transpose()?.unwrap_or(0);
        for entry in page {
            let doc=entry.get("document").ok_or_else(conflict)?;
            let path:PathBuf=serde_json::from_value(doc.get("path").cloned().ok_or_else(conflict)?)?;
            let descriptor=PathBuf::from(format!("{}.sealed.json",path.display()));
            controlled_file(&self.paths.media,&descriptor)?;
            if std::fs::metadata(&descriptor)?.len()>4096 {return Err(conflict());}
            let bytes=std::fs::read(&descriptor)?;
            if serde_json::from_slice::<Value>(&bytes)?!=*doc {return Err(conflict());}
            let digest=hex::encode(Sha256::digest(&bytes));
            if entry.get("descriptor_sha256").and_then(Value::as_str)!=Some(&digest) {return Err(conflict());}
            let mut metadata:CaptureSegmentIdentity=serde_json::from_value({let mut v=doc.clone();v["descriptor_sha256"]=json!(digest);v})?;
            if metadata.session_id!=session || metadata.instance_id!=origin || metadata.sequence!=expected || metadata.sample_rate!=48000 || metadata.frame_count==0 || (metadata.frame_count>1800*48000 && !metadata.legacy_wav) {return Err(conflict());}
            if metadata.legacy_wav && (metadata.sequence!=0 || metadata.offset_frames!=0 || doc["temporary"]!=false) {return Err(conflict());}
            let sha=entry.get("sha256").and_then(Value::as_str).filter(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit())).ok_or_else(conflict)?.to_owned();
            let mut audio=if path.exists() || std::fs::symlink_metadata(&path).is_ok() {
                let sealed=self.seal_files(&json!({"files":[path]})).await?;
                let a=sealed.into_iter().next().ok_or_else(conflict)?;
                let wav=hound::WavReader::open(&a.path).map_err(|_|conflict())?;
                if a.sha256!=sha || u64::from(wav.duration())!=metadata.frame_count {return Err(conflict());}
                a
            } else {
                // Admission accepts this only when its original immutable DB row already exists.
                canonical_private(path.parent().ok_or_else(conflict)?)?;
                AudioSegment{path,sha256:sha,duration_ms:metadata.frame_count*1000/48000,capture:None}
            };
            metadata.descriptor_sha256=digest;audio.capture=Some(metadata);
            let seen=self.sealed_seen.lock().map_err(|_|conflict())?;
            if let Some(old)=seen.get(&audio.path) {
                if serde_json::to_value(old)?!=serde_json::to_value(&audio)? { return Err(conflict()); }
            } else if !audio.path.exists() { return Err(conflict()); }
            drop(seen);result.push(audio);expected=expected.checked_add(1).ok_or_else(conflict)?;
        }
        Ok((result,value.get("has_more").and_then(Value::as_bool).ok_or_else(conflict)?))
    }
    async fn seal_files(&self, value: &Value) -> Result<Vec<AudioSegment>> {
        let paths: Vec<PathBuf> =
            serde_json::from_value(value.get("files").cloned().ok_or_else(|| {
                OmmError::new(ErrorCode::Corrupt, false, "capture stop lacks segment list")
            })?)?;
        if paths.len() > 4096 {
            return Err(OmmError::invalid("too many capture segments"));
        }
        let root = self.paths.media.clone();
        tokio::task::spawn_blocking(move || {
            let mut segments = vec![];
            let mut seen = std::collections::HashSet::new();
            for path in paths {
                controlled_file(&root, &path)?;
                if !seen.insert(path.clone()) {
                    return Err(OmmError::invalid("duplicate capture segment"));
                }
                let reader = hound::WavReader::open(&path).map_err(|_| {
                    OmmError::new(ErrorCode::Corrupt, false, "capture WAV is invalid")
                })?;
                let spec = reader.spec();
                if spec.channels != 2
                    || spec.sample_rate != 48000
                    || spec.bits_per_sample != 16
                    || spec.sample_format != hound::SampleFormat::Int
                {
                    return Err(OmmError::new(
                        ErrorCode::Corrupt,
                        false,
                        "capture WAV format differs from dual-track contract",
                    ));
                }
                let duration_ms = u64::from(reader.duration()) * 1000 / 48000;
                if reader.duration() == 0 {
                    continue;
                }
                let declared_bytes = u64::from(reader.len()) * 2;
                let offset = reader.into_inner().stream_position()?;
                if std::fs::metadata(&path)?.len() < offset + declared_bytes {
                    return Err(OmmError::new(
                        ErrorCode::Corrupt,
                        false,
                        "capture WAV data is truncated",
                    ));
                }
                let mut file = std::fs::File::open(&path)?;
                let mut hash = Sha256::new();
                let mut buffer = [0u8; 65536];
                loop {
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    hash.update(&buffer[..n]);
                }
                segments.push(AudioSegment { capture: None,
                    path,
                    duration_ms,
                    sha256: hex::encode(hash.finalize()),
                });
            }
            Ok(segments)
        })
        .await
        .map_err(|_| OmmError::new(ErrorCode::Backend, false, "capture seal worker failed"))?
    }
}
#[async_trait]
impl CaptureAdapter for MacCaptureAdapter {
    fn supports_rolling(&self) -> bool {
        true
    }
    async fn sealed_segments(&self) -> Result<Vec<AudioSegment>> {
        let _guard = self.operation.lock().await;
        let status = self.status_inner().await?;
        let session = status.session_id.ok_or_else(conflict)?;
        let cursor = self.sealed_cursor()?;
        Ok(self.sealed_page(session, cursor).await?.0)
    }
    fn acknowledge_segments(&self, segments: &[AudioSegment]) -> Result<()> {
        let mut seen = self.sealed_seen.lock().map_err(|_| OmmError::unavailable("capture segment cache unavailable"))?;
        for audio in segments {
            if let Some(old) = seen.get(&audio.path) {
                if serde_json::to_value(old)? != serde_json::to_value(audio)? { return Err(conflict()); }
            } else { seen.insert(audio.path.clone(),audio.clone()); }
        }
        Ok(())
    }
    async fn start(&self, req: CaptureRequest) -> Result<CaptureStatus> {
        let _guard = self.operation.lock().await;
        self.paths.validate()?;
        canonical_private(&req.context.meeting_dir)?;
        canonical_private(&req.context.work_dir)?;
        if !req.context.meeting_dir.starts_with(&self.paths.media)
            || req.output_path.parent() != Some(req.context.meeting_dir.as_path())
            || req.output_path.exists()
            || std::fs::symlink_metadata(&req.output_path).is_ok()
            || req.context.lease.expires_at_ms <= chrono::Utc::now().timestamp_millis()
        {
            return Err(OmmError::invalid("capture output or lease is invalid"));
        }
        let filename = req
            .output_path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| s.ends_with(".wav") && *s != ".wav")
            .ok_or_else(|| OmmError::invalid("capture output must be a WAV filename"))?;
        if self.status_inner().await?.recording {
            return Err(conflict());
        }
        let value = self.exchange(json!({"action": "start", "session_id": req.context.session_id,
            "instance_id": self.instance_id, "filename": filename, "output_dir": req.context.meeting_dir,
            "title": "meeting", "segment_ms": req.segment_ms.clamp(60_000,1_800_000)})).await?;
        self.instance_matches(&value)?;
        if value.get("status").and_then(Value::as_str) != Some("recording")
            || value.get("file").and_then(Value::as_str) != req.output_path.to_str()
            || value.get("session_id").and_then(Value::as_str)
                != Some(req.context.session_id.to_string().as_str())
        {
            return Err(conflict());
        }
        let status = self.status_inner().await?;
        if !status.recording
            || status.path.as_ref() != Some(&req.output_path)
            || status.session_id != Some(req.context.session_id)
        {
            return Err(conflict());
        }
        self.sealed_seen.lock().map_err(|_| conflict())?.clear();
        Ok(status)
    }
    async fn stop(&self) -> Result<CaptureStop> {
        let _guard = self.operation.lock().await;
        let status = self.status_inner().await?;
        let Some(session_id) = status.session_id else {
            if let Some(error) = status.errors.into_iter().find(|e| e.code == ErrorCode::Io) {
                return Err(error);
            }
            return Ok(CaptureStop {
                session_id: None,
                segments: vec![],
                duration_ms: 0,
            });
        };
        let value = self.exchange(json!({"action": "stop", "session_id": session_id, "instance_id": self.instance_id})).await?;
        self.instance_matches(&value)?;
        if value.get("status").and_then(Value::as_str) != Some("stopped")
            || value.get("session_id").and_then(Value::as_str)
                != Some(session_id.to_string().as_str())
        {
            return Err(conflict());
        }
        let segments = if value.get("paged_segments").and_then(Value::as_bool) == Some(true) { self.all_sealed(session_id).await? } else { self.seal_files(&value).await? };
        let duration_ms = segments.iter().map(|s| s.duration_ms).sum();
        Ok(CaptureStop {
            session_id: Some(session_id),
            segments,
            duration_ms,
        })
    }
    async fn status(&self) -> Result<CaptureStatus> {
        let _guard = self.operation.lock().await;
        self.status_inner().await
    }
    async fn windows(&self) -> Result<Vec<WindowInfo>> {
        let value = self.exchange(json!({"action": "windows"})).await?;
        if value.get("busy").and_then(Value::as_bool) == Some(true) {
            return Err(OmmError::new(
                ErrorCode::Unavailable,
                true,
                "capture window scan busy",
            ));
        }
        let rows = value
            .get("windows")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                OmmError::new(ErrorCode::Corrupt, false, "capture window list invalid")
            })?;
        rows.iter()
            .map(|row| {
                Ok(WindowInfo {
                    id: 0, // Original AX helper does not expose native window IDs; 0 means unknown.
                    title: field(row, "title")?,
                    app: field(row, "app")?,
                })
            })
            .collect()
    }
    async fn devices(&self) -> Result<Vec<AudioDevice>> {
        let value = self.exchange(json!({"action": "audio_devices"})).await?;
        let rows = value
            .get("input")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                OmmError::new(ErrorCode::Corrupt, false, "capture device list invalid")
            })?;
        rows.iter()
            .map(|row| {
                let id = field(row, "uid")?;
                Ok(AudioDevice {
                    is_default: value.get("default_input_uid").and_then(Value::as_str)
                        == Some(id.as_str()),
                    id,
                    name: field(row, "name")?,
                })
            })
            .collect()
    }
}
fn field(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| OmmError::new(ErrorCode::Corrupt, false, "capture response field missing"))
}

async fn exchange_connected(stream: &mut UnixStream, bytes: &[u8]) -> Result<Value> {
    stream.write_all(bytes).await?;
    // Swift waits for EOF, not a newline. Keep the read half alive.
    stream.shutdown().await?;
    let mut response = Vec::new();
    stream
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut response)
        .await?;
    if response.len() as u64 > MAX_RESPONSE {
        return Err(OmmError::new(
            ErrorCode::Corrupt,
            false,
            "capture response exceeds limit",
        ));
    }
    let value: Value = serde_json::from_slice(&response)?;
    if value.get("error").is_some() {
        return Err(backend_error(&value));
    }
    Ok(value)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SocketIdentity {
    dev: u64,
    ino: u64,
}
impl SocketIdentity {
    fn read(path: &Path) -> Result<Option<Self>> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if !metadata.file_type().is_socket()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(OmmError::new(
                ErrorCode::Permission,
                false,
                "capture socket is not private",
            ));
        }
        Ok(Some(Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
        }))
    }
}
#[cfg(target_os = "macos")]
async fn shutdown_connect(socket: &Path, identity: SocketIdentity) -> Result<UnixStream> {
    if SocketIdentity::read(socket)? != Some(identity) {
        return Err(conflict());
    }
    let stream = UnixStream::connect(socket)
        .await
        .map_err(|_| OmmError::unavailable("capture helper unreachable; exit not confirmed"))?;
    if stream.peer_cred()?.uid() != unsafe { libc::geteuid() }
        || SocketIdentity::read(socket)? != Some(identity)
    {
        return Err(conflict());
    }
    Ok(stream)
}

#[cfg(target_os = "macos")]
mod shutdown_process;

/// OS microphone status is public; system-audio authorization remains unverified.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturePermissions {
    pub instance_id: Uuid,
    pub request_id: Uuid,
    pub microphone: String,
    #[serde(rename = "systemAudio")]
    pub system_audio: String,
    pub outcome: String,
}

/// Core authenticates local transport and reconciles this against status before stop.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureAutoStop {
    pub schema_version: u32,
    pub command: String,
    pub session_id: Uuid,
    pub instance_id: Uuid,
}
impl CaptureAutoStop {
    pub fn validate(&self, expected_instance: Uuid, status: &CaptureStatus) -> Result<()> {
        if self.schema_version != 1 || self.command != "capture_auto_stop" {
            return Err(OmmError::invalid("invalid capture callback"));
        }
        if self.instance_id != expected_instance
            || status.session_id != Some(self.session_id)
            || !status.recording
        {
            return Err(conflict());
        }
        Ok(())
    }
}

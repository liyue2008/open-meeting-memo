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
        | "application_transaction_in_progress" => conflict(),
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
            shutdown_confirmed: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub fn socket_path(&self) -> PathBuf {
        self.paths.ipc.join("audio_daemon.sock")
    }

    /// Explicit lifecycle action only. Core owns child monitoring and reaping.
    /// Launch may resume an interrupted recording; never call it as a status probe.
    pub fn launch_helper(&self, executable: &Path, service_socket: &Path) -> Result<Child> {
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
                segments.push(AudioSegment {
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
            "title": "meeting"})).await?;
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
        let segments = self.seal_files(&value).await?;
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

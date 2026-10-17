//! Explicit profile/bootstrap and service lifecycle. Merely observing never opens a writable store.
use super::{bundle::Bundle, profile};
use crate::{
    core::{
        config::AppConfig,
        contracts::*,
        paths::*,
        runtime::{self, RuntimeOwner},
        store::{hash_bytes, hash_file, Store},
    },
    service::daemon,
};
use serde_json::{json, Value};
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::PathBuf, process::Stdio};
use uuid::Uuid;
#[derive(Clone)]
pub struct Plan {
    pub token: String,
    pub action: String,
    pub snapshot: String,
    pub bundle_hash: String,
    pub expires: i64,
}
fn changed() -> OmmError {
    OmmError::new(
        ErrorCode::Conflict,
        false,
        "service plan changed or expired; inspect a new plan",
    )
}
fn location(paths: &Paths) -> Result<runtime::RuntimeLocation> {
    paths.runtime_location(&std::env::vars().collect())
}
fn empty(paths: &Paths) -> Result<bool> {
    for root in [&paths.config, &paths.data, &paths.state] {
        if root.try_exists()? {
            validate_private_dir(root)?;
            for entry in std::fs::read_dir(root)? {
                let name = entry?.file_name();
                if root == &paths.state && name == "service.lock"
                    || root == &paths.config
                        && ["profile-writers.lock", "desktop-lifecycle.lock"]
                            .iter()
                            .any(|n| name == *n)
                {
                    continue;
                }
                return Ok(false);
            }
        }
    }
    Ok(true)
}
fn lifecycle_lock(paths: &Paths) -> Result<std::fs::File> {
    private_dir(&paths.config)?;
    let path = paths.config.join("desktop-lifecycle.lock");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    validate_private_file(&path)?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| changed())?;
    Ok(file)
}
pub fn initialize(paths: &Paths, bundle: &Bundle) -> Result<()> {
    let _lifecycle = lifecycle_lock(paths)?;
    if !empty(paths)? {
        return Err(changed());
    }
    bundle.verify()?;
    paths.create_private_dirs()?;
    let guard = runtime::RuntimeGuard::acquire(&location(paths)?, &paths.state)?;
    let _writer = profile::writer_lock(paths)?;
    if !empty(paths)? {
        return Err(changed());
    }
    let mut cfg = AppConfig {
        capture_executable: bundle.file("OMMCapture.app/Contents/MacOS/omm-capture")?,
        worker_executable: bundle.file("bin/omm-asr-worker")?,
        ..Default::default()
    };
    cfg.asr.library_path = bundle.file("sherpa/libsherpa-onnx-c-api.dylib")?;
    // Models stay external, manifest-bound and absent until the explicit model install operation.
    let models = paths.data.join("models").join(&cfg.asr.model_version);
    cfg.asr.encoder_path = models.join("encoder.int8.onnx");
    cfg.asr.decoder_path = models.join("decoder.int8.onnx");
    cfg.asr.tokens_path = models.join("tokens.txt");
    cfg.asr.vad_path = models.join("silero_vad.onnx");
    cfg.notifications.enabled = false;
    let owner = profile::ProfileOwner {
        schema_version: 1,
        min_reader_version: 1,
        generation: guard.owner.instance_id,
        profile_id: profile::profile_id(paths)?,
    };
    // Config publish is last: an interrupted bootstrap remains visibly incomplete rather than overwriting it.
    write_private_new(
        &paths.state.join("desktop-owner.json"),
        &serde_json::to_vec(&owner)?,
    )?;
    crate::core::audio_toolchain::install(paths, &bundle.resources.join("audio-toolchain"))?;
    cfg.save(paths)?;
    Ok(())
}
#[cfg(target_os = "macos")]
fn process_path(pid: i32) -> Result<PathBuf> {
    let mut buf = vec![0u8; 4096];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return Err(changed());
    }
    buf.truncate(n as usize);
    if buf.last() == Some(&0) {
        buf.pop();
    }
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_vec(buf)).canonicalize()?)
}
#[cfg(not(target_os = "macos"))]
fn process_path(_pid: i32) -> Result<PathBuf> {
    Err(OmmError::unavailable("macOS service identity required"))
}
fn owned_service(owner: &RuntimeOwner, bundle: &Bundle) -> Result<bool> {
    let executable = process_path(owner.pid)?;
    if executable == bundle.cli()? {
        return Ok(hash_file(&executable)? == bundle.manifest.files["bin/omm"]);
    }
    let resources = executable
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(changed)?;
    let old = Bundle::open(resources)?;
    Ok(old.cli()? == executable)
}
async fn inspect(paths: &Paths, bundle: Option<&Bundle>, action: &str) -> Result<(Value, String)> {
    let owner = profile::read(paths)?;
    let runtime = location(paths)?;
    let service = runtime::verify_active(&runtime.path, &paths.state).ok();
    let status = if service.is_some() {
        daemon::request(paths, "status", None).await.ok()
    } else {
        None
    };
    let initialized = owner.is_some() && paths.config.join("config.json").try_exists()?;
    let mut compatibility = owner
        .as_ref()
        .map(|o| profile::compatible(o, "offline"))
        .unwrap_or_else(|| profile::incompatible("unknown", "incompatible_schema", true));
    let mut state = if initialized {
        "stopped"
    } else {
        "uninitialized"
    };
    let mut reasons = vec![];
    let mut apply = false;
    if let Some(service) = &service {
        state = "unknown";
        if status.is_none() {
            reasons.push("service_unreachable");
        }
        let owned = bundle.is_some_and(|b| owned_service(service, b).unwrap_or(false));
        if !owned {
            reasons.push("unowned_service");
        } else if let Some(status) = &status {
            let capability = daemon::request_message_at(
                paths,
                &runtime,
                daemon::Request {
                    schema_version: 1,
                    command: "desktop_capabilities_v1".into(),
                    title: None,
                    session_id: None,
                    instance_id: Some(service.instance_id),
                    desktop_request: None,
                    desktop_capability: Some(profile::capability_request()),
                },
            )
            .await
            .ok();
            if let Some(value) = capability {
                compatibility = value["compatibility"].clone();
                state = "running";
            } else {
                state = "incompatible";
                reasons.push("incompatible_service");
            }
            let store = Store::open_read_only(paths)?;
            let busy = status["recording"].is_object()
                || status["state"] == "recording"
                || store.state::<Value>("recording")?.is_some()
                || store.tasks()?.iter().any(|t| {
                    matches!(t.status, TaskStatus::Transcribing | TaskStatus::Summarizing)
                });
            if busy {
                state = "busy";
                reasons.push("service_busy");
            } else if action == "start" {
                reasons.push("already_running");
            } else if state == "running" {
                apply = true;
            }
        }
    } else if profile::offline_lock(paths).is_err() {
        state = "unknown";
        reasons.push("service_unreachable");
    } else if !initialized {
        reasons.push(if empty(paths)? {
            "profile_not_initialized"
        } else {
            "profile_not_empty"
        });
    } else if action == "stop" {
        reasons.push("already_stopped");
    } else {
        apply = true;
    }
    if bundle.is_none() {
        reasons.push("bundle_unavailable");
        apply = false;
    }
    if state == "busy" || state == "unknown" {
        apply = false;
    }
    if reasons.is_empty() {
        reasons.push("ready");
    }
    let config_path = paths.config.join("config.json");
    let config = if config_path.try_exists()? {
        validate_private_file(&config_path)?;
        Some(hash_file(&config_path)?)
    } else {
        None
    };
    let snapshot = hash_bytes(&serde_json::to_vec(
        &json!({"profile":profile::profile_id(paths)?,"owner":owner,"service":service,"status":status,"config":config,"state":state,"reasons":reasons}),
    )?);
    Ok((
        json!({"action":action,"state":state,"canInitialize":!initialized&&empty(paths)?&&service.is_none(),"canApply":apply,"planToken":null,"expiresAtMs":null,"targetVersion":env!("CARGO_PKG_VERSION"),"reasons":reasons,"compatibility":compatibility}),
        snapshot,
    ))
}
pub async fn plan(
    paths: &Paths,
    bundle: Option<&Bundle>,
    action: &str,
) -> Result<(Value, Option<Plan>)> {
    if !["start", "stop", "switch"].contains(&action) {
        return Err(OmmError::invalid("unsupported service action"));
    }
    let (mut value, snapshot) = inspect(paths, bundle, action).await?;
    let plan = if value["canApply"] == true {
        let plan = Plan {
            token: Uuid::new_v4().simple().to_string(),
            action: action.into(),
            snapshot,
            bundle_hash: bundle.unwrap().manifest_hash.clone(),
            expires: chrono::Utc::now().timestamp_millis() + 60000,
        };
        value["planToken"] = json!(plan.token);
        value["expiresAtMs"] = json!(plan.expires);
        Some(plan)
    } else {
        None
    };
    Ok((value, plan))
}
pub async fn apply(paths: &Paths, bundle: &Bundle, plan: Plan) -> Result<Value> {
    let _lifecycle = lifecycle_lock(paths)?;
    if plan.expires < chrono::Utc::now().timestamp_millis()
        || plan.bundle_hash != bundle.manifest_hash
    {
        return Err(changed());
    }
    bundle.verify()?;
    let (value, snapshot) = inspect(paths, Some(bundle), &plan.action).await?;
    if snapshot != plan.snapshot || value["canApply"] != true {
        return Err(changed());
    }
    let runtime = location(paths)?;
    if let Ok(owner) = runtime::verify_active(&runtime.path, &paths.state) {
        if !owned_service(&owner, bundle)? {
            return Err(changed());
        }
        daemon::request_message_at(
            paths,
            &runtime,
            daemon::Request {
                schema_version: 1,
                command: "desktop_idle_shutdown_v1".into(),
                title: None,
                session_id: None,
                instance_id: Some(owner.instance_id),
                desktop_capability: None,
                desktop_request: None,
            },
        )
        .await?;
        for _ in 0..200 {
            if !runtime::owner_is_live(&owner) && profile::offline_lock(paths).is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        if runtime::owner_is_live(&owner) {
            return Err(OmmError::new(
                ErrorCode::Timeout,
                false,
                "service drain did not finish; no replacement started",
            ));
        }
    }
    if plan.action == "stop" {
        let owner = profile::read(paths)?.ok_or_else(changed)?;
        return Ok(
            json!({"action":"stop","state":"stopped","compatibility":profile::compatible(&owner,"offline")}),
        );
    }
    // RuntimeGuard is the service's exclusive startup admission. A competing start wins that lock, never gets killed.
    {
        let _guard = runtime::RuntimeGuard::acquire(&runtime, &paths.state)?;
    }
    let mut command = std::process::Command::new(bundle.cli()?);
    command
        .args(["--json", "service", "run"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, path) in [
        ("XDG_CONFIG_HOME", &paths.config),
        ("XDG_DATA_HOME", &paths.data),
        ("XDG_STATE_HOME", &paths.state),
        ("XDG_CACHE_HOME", &paths.cache),
    ] {
        command.env(key, path.parent().ok_or_else(changed)?);
    }
    let mut child = command.spawn()?;
    let pid = child.id() as i32;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    for _ in 0..200 {
        if let Ok(owner) = runtime::verify_active(&runtime.path, &paths.state) {
            if owner.pid != pid || !owned_service(&owner, bundle)? {
                return Err(changed());
            }
            if let Some(profile) = profile::read(paths)? {
                return Ok(
                    json!({"action":plan.action,"state":"running","compatibility":profile::compatible(&profile,"service")}),
                );
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err(OmmError::new(
        ErrorCode::Timeout,
        false,
        "service startup did not become observable; inspect status before retry",
    ))
}

//! Explicit profile/bootstrap and service lifecycle. Merely observing never opens a writable store.
use super::{bundle::Bundle, profile, recovery};
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
    pub recovery: Option<recovery::RecoverySnapshot>,
    pub offline_snapshot: Option<String>,
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
pub fn owned_service(owner: &RuntimeOwner, bundle: &Bundle) -> Result<bool> {
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
fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| OmmError::invalid("HOME is required"))
}
pub const EXTERNAL_STOP_GUIDANCE: &str = "The original service cannot be verified. Use the original CLI executable from your existing installation with `service stop` (not PATH's `omm service start`), wait for normal seal and child exit, then inspect Switch again. The App has not stopped it.";
fn external_stop_guidance(owner: &RuntimeOwner) -> String {
    let mut guidance = EXTERNAL_STOP_GUIDANCE.to_owned();
    if let Ok(path) = process_path(owner.pid) {
        if let Some(path) = path.to_str() {
            let quoted = format!("'{}'", path.replace('\'', "'\"'\"'"));
            guidance.push_str(&format!(" Observed executable path (not a trust assertion): {}. Only after you verify that this is your original installation, manually run: {} service stop",serde_json::to_string(path).unwrap_or_default(),quoted));
        }
    }
    guidance
}
pub const ADOPTION_GUIDANCE: &str = "Switch only adopts the stopped profile and updates its exact service registration. It does not start a service. After adoption, inspect Start and explicitly confirm startup.";
async fn inspect(
    paths: &Paths,
    bundle: Option<&Bundle>,
    action: &str,
    home: &std::path::Path,
    runtime: &runtime::RuntimeLocation,
    runner: &dyn crate::service::registration::ServiceRunner,
) -> Result<(Value, String)> {
    let owner_result = profile::read(paths);
    let owner = owner_result.as_ref().ok().and_then(|o| o.as_ref());
    let service = runtime::verify_active(&runtime.path, &paths.state).ok();
    let initialized = owner.is_some() && paths.config.join("config.json").try_exists()?;
    let mut compatibility = owner
        .map(|o| profile::compatible(o, "offline"))
        .unwrap_or_else(|| {
            profile::incompatible(
                "requires_switch",
                if owner_result.is_err() {
                    "incompatible_schema"
                } else {
                    "profile_requires_takeover"
                },
                true,
            )
        });
    let mut state = if initialized {
        "stopped"
    } else {
        "uninitialized"
    };
    let mut reasons = vec![];
    let mut apply = false;
    let mut status = None;
    let mut effects = vec![];
    let mut guidance = None;
    let mut recovery_snapshot = None;
    let mut database_snapshot = None;
    if let Some(service) = &service {
        state = "unknown";
        let owned = bundle.is_some_and(|b| owned_service(service, b).unwrap_or(false));
        if !owned {
            reasons.extend(["unowned_service", "external_stop_required"]);
            guidance = Some(external_stop_guidance(service));
        } else {
            let capability = daemon::request_message_at(
                paths,
                runtime,
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
            if let Some(value) = capability.filter(|v| {
                v["compatibility"]["serviceGeneration"] == service.instance_id.to_string()
                    && v["compatibility"]["state"] == "compatible"
            }) {
                compatibility = value["compatibility"].clone();
                status = super::observation::service_status(paths, runtime, service).ok();
                if let Some(status) = &status {
                    if !status
                        .get("recording")
                        .is_some_and(|r| r.is_null() || r.is_object())
                    {
                        reasons.push("service_unreachable");
                    } else {
                        for suffix in ["-wal", "-shm", "-journal"] {
                            if paths
                                .state
                                .join(format!("state.sqlite{suffix}"))
                                .try_exists()?
                            {
                                return Err(changed());
                            }
                        }
                        let store = Store::open_read_only(paths)?;
                        let busy = status["recording"].is_object()
                            || store.state::<Value>("recording")?.is_some()
                            || store.has_unsealed_segmented_sessions()?
                            || store.desktop_processing_busy()?;
                        if busy {
                            state = "busy";
                            reasons.push("service_busy");
                        } else {
                            state = "running";
                            if action == "start" {
                                reasons.push("already_running");
                            } else {
                                apply = true;
                            }
                        }
                    }
                } else {
                    reasons.push("service_unreachable");
                }
            } else {
                state = "incompatible";
                reasons.extend(["incompatible_service", "external_stop_required"]);
                guidance = Some(external_stop_guidance(service));
            }
        }
    } else if owner_result.is_err() {
        state = "incompatible";
        reasons.push("incompatible_schema");
    } else if !initialized && !empty(paths)? {
        if let Some(bundle) = bundle {
            let checked = match runner.require_unloaded().await {
                Ok(()) => recovery::inspect(paths, home, runtime, bundle),
                Err(error) => Err(error),
            };
            match checked {
                Ok(snapshot) => {
                    state = "stopped";
                    reasons.push("profile_requires_takeover");
                    apply = action == "switch";
                    recovery_snapshot = Some(snapshot.fingerprint);
                    guidance = Some(ADOPTION_GUIDANCE.to_owned());
                    effects=vec!["Adopt this stopped profile under the current ownership contract","Replace only the exact service registration executable with the verified bundled CLI; retain a non-secret metadata backup","Keep configuration, consent and database bytes unchanged; do not start a service","Next: explicitly inspect and confirm Start; startup may add supported Store tables"] ;
                }
                Err(error) => {
                    state = if error.message.contains("schema is unsupported") {
                        "incompatible"
                    } else {
                        "unknown"
                    };
                    reasons.push(if state == "incompatible" {
                        "incompatible_schema"
                    } else if error.message
                        == crate::service::registration::recovery::EXTERNAL_UNLOAD_REQUIRED
                    {
                        "external_unload_required"
                    } else {
                        "registration_review_required"
                    });
                    guidance=Some(format!("Stopped recovery could not be verified; no files changed. For a loaded old registration, only after verifying that the original tool and current-user job are your OMM and all work is safely completed: use the verified original CLI's service stop and wait for success; manually verify the loaded job's identity, then manually run /bin/launchctl bootout 'gui/{}/org.openmeetingmemo.service'. This unloads only the user job and keeps its plist. Do not use service uninstall: it removes the plist needed for this recovery. If ownership is uncertain, seek manual review instead. Return to inspect Switch after the job is unloaded; adoption and Start are separate confirmations.",unsafe{libc::geteuid()}));
                }
            }
        }
    } else if !initialized {
        reasons.push("profile_not_initialized");
    } else if recovery::existing_lock(&paths.state.join("service.lock")).is_err() {
        state = "unknown";
        reasons.push("service_unreachable");
    } else if action == "stop" {
        reasons.push("already_stopped");
    } else if let Some(bundle) = bundle {
        match recovery::stopped_start_snapshot(paths, home, runtime, bundle) {
            Ok(fingerprint) => {
                database_snapshot = Some(fingerprint);
                apply = true;
            }
            Err(error) => {
                state = if error.message.contains("schema is unsupported") {
                    "incompatible"
                } else {
                    "unknown"
                };
                reasons.push(if state == "incompatible" {
                    "incompatible_schema"
                } else {
                    "registration_review_required"
                });
            }
        }
    }
    if bundle.is_none() {
        reasons.push("bundle_unavailable");
        apply = false;
    }
    if state == "busy" || state == "unknown" || state == "incompatible" {
        apply = false;
    }
    if reasons.is_empty() {
        reasons.push("ready");
    }
    if effects.is_empty() && apply {
        effects.push(if action == "stop" {
            "Drain and stop the verified idle service"
        } else {
            "Explicitly start the verified bundled service; startup may add supported Store tables"
        });
    }
    let config_path = paths.config.join("config.json");
    let config = if config_path.try_exists()? {
        validate_private_file(&config_path)?;
        Some(hash_file(&config_path)?)
    } else {
        None
    };
    let snapshot = hash_bytes(&serde_json::to_vec(
        &json!({"profile":profile::profile_id(paths)?,"owner":owner,"service":service,"status":status,"config":config,"state":state,"reasons":reasons,"recovery":recovery_snapshot,"database":database_snapshot}),
    )?);
    Ok((
        json!({"action":action,"state":state,"canInitialize":!initialized&&empty(paths)?&&service.is_none(),"canApply":apply,"planToken":null,"expiresAtMs":null,"targetVersion":env!("CARGO_PKG_VERSION"),"reasons":reasons,"compatibility":compatibility,"effects":effects,"guidance":guidance}),
        snapshot,
    ))
}
pub async fn plan(
    paths: &Paths,
    bundle: Option<&Bundle>,
    action: &str,
) -> Result<(Value, Option<Plan>)> {
    plan_at(
        paths,
        bundle,
        action,
        &home()?,
        &location(paths)?,
        &crate::service::registration::Launchctl,
    )
    .await
}
pub async fn plan_at(
    paths: &Paths,
    bundle: Option<&Bundle>,
    action: &str,
    home: &std::path::Path,
    runtime: &runtime::RuntimeLocation,
    runner: &dyn crate::service::registration::ServiceRunner,
) -> Result<(Value, Option<Plan>)> {
    if !["start", "stop", "switch"].contains(&action) {
        return Err(OmmError::invalid("unsupported service action"));
    }
    let (mut value, snapshot) = inspect(paths, bundle, action, home, runtime, runner).await?;
    let plan = if value["canApply"] == true {
        let plan = Plan {
            token: Uuid::new_v4().simple().to_string(),
            action: action.into(),
            snapshot,
            bundle_hash: bundle.unwrap().manifest_hash.clone(),
            expires: chrono::Utc::now().timestamp_millis() + 60000,
            offline_snapshot: if value["state"] == "stopped"
                && !value["reasons"]
                    .as_array()
                    .is_some_and(|r| r.iter().any(|r| r == "profile_requires_takeover"))
            {
                Some(recovery::stopped_start_snapshot(
                    paths,
                    home,
                    runtime,
                    bundle.unwrap(),
                )?)
            } else {
                None
            },
            recovery: if value["reasons"]
                .as_array()
                .is_some_and(|r| r.iter().any(|r| r == "profile_requires_takeover"))
            {
                Some(recovery::inspect(paths, home, runtime, bundle.unwrap())?)
            } else {
                None
            },
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
    apply_at(
        paths,
        bundle,
        plan,
        &home()?,
        &location(paths)?,
        &crate::service::registration::Launchctl,
    )
    .await
}
pub async fn apply_at(
    paths: &Paths,
    bundle: &Bundle,
    plan: Plan,
    home: &std::path::Path,
    runtime: &runtime::RuntimeLocation,
    runner: &dyn crate::service::registration::ServiceRunner,
) -> Result<Value> {
    let _lifecycle = lifecycle_lock(paths)?;
    if plan.expires < chrono::Utc::now().timestamp_millis()
        || plan.bundle_hash != bundle.manifest_hash
    {
        return Err(changed());
    }
    bundle.verify()?;
    let (value, snapshot) =
        inspect(paths, Some(bundle), &plan.action, home, runtime, runner).await?;
    if snapshot != plan.snapshot || value["canApply"] != true {
        return Err(changed());
    }
    if let Some(recovery_plan) = &plan.recovery {
        recovery::adopt(paths, home, runtime, bundle, recovery_plan, runner).await?;
        let owner = profile::read(paths)?.ok_or_else(changed)?;
        return Ok(
            json!({"action":"switch","state":"stopped","compatibility":profile::compatible(&owner,"offline")}),
        );
    }
    if let Ok(owner) = runtime::verify_active(&runtime.path, &paths.state) {
        if !owned_service(&owner, bundle)? {
            return Err(changed());
        }
        daemon::request_message_at(
            paths,
            runtime,
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
        let _guard = runtime::RuntimeGuard::acquire(runtime, &paths.state)?;
        let _writer = profile::writer_lock(paths)?;
        let _config_writer = crate::core::config_transaction::ConfigTransaction::acquire(paths)?;
        bundle.verify()?;
        let current = recovery::start_snapshot(paths, home, runtime, bundle, true)?;
        if plan
            .offline_snapshot
            .as_ref()
            .is_some_and(|expected| expected != &current)
        {
            return Err(changed());
        }
    }
    let mut command = std::process::Command::new(bundle.cli()?);
    command
        .args(["--json", "service", "run"])
        .env("HOME", home)
        .env(
            "XDG_RUNTIME_DIR",
            runtime.path.parent().ok_or_else(changed)?,
        )
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
                if profile.generation == owner.instance_id {
                    if let Ok(capabilities) = daemon::request_message_at(
                        paths,
                        runtime,
                        daemon::Request {
                            schema_version: 1,
                            command: "desktop_capabilities_v1".into(),
                            title: None,
                            session_id: None,
                            instance_id: Some(owner.instance_id),
                            desktop_request: None,
                            desktop_capability: Some(profile::capability_request()),
                        },
                    )
                    .await
                    {
                        if capabilities["compatibility"]["state"] == "compatible"
                            && capabilities["compatibility"]["serviceGeneration"]
                                == profile.generation.to_string()
                        {
                            return Ok(
                                json!({"action":plan.action,"state":"running","compatibility":profile::compatible(&profile,"service")}),
                            );
                        }
                    }
                }
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

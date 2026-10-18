use super::ServiceProfile;
use crate::core::{contracts::*, paths::*};
use async_trait::async_trait;
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
mod files;
pub mod recovery;
use files::{Directory, Snapshot};
const LABEL: &str = "org.openmeetingmemo.service";
const PLIST: &str = "org.openmeetingmemo.service.plist";
const BACKUP: &str = "org.openmeetingmemo.service.pre-standard.backup";
fn plist(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
#[async_trait]
pub trait ServiceRunner: Send + Sync {
    async fn run(&self, args: &[String]) -> Result<()>;
    async fn require_unloaded(&self) -> Result<()> {
        Err(OmmError::unavailable(
            "service registration state cannot be verified",
        ))
    }
}
pub struct Launchctl;
#[async_trait]
impl ServiceRunner for Launchctl {
    async fn require_unloaded(&self) -> Result<()> {
        use tokio::io::AsyncReadExt;
        let uid = unsafe { libc::geteuid() };
        if uid == 0 {
            return Err(OmmError::unavailable("user service registration required"));
        }
        let mut child = tokio::process::Command::new("/bin/launchctl")
            .args(["print", &format!("gui/{uid}/{LABEL}")])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| OmmError::unavailable("service registration state unavailable"))?;
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut bytes = Vec::new();
            stderr.take(4097).read_to_end(&mut bytes).await?;
            if bytes.len() > 4096 {
                return Err(OmmError::unavailable(
                    "service registration state exceeds bound",
                ));
            }
            let status = child.wait().await?;
            recovery::require_unloaded_output(uid, status.code(), &bytes)
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            other => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                match other {
                    Ok(Err(error)) => Err(error),
                    _ => Err(OmmError::unavailable(
                        "service registration check timed out",
                    )),
                }
            }
        }
    }
    async fn run(&self, args: &[String]) -> Result<()> {
        let status = tokio::process::Command::new("/bin/launchctl")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map_err(|_| OmmError::unavailable("user service manager unavailable"))?;
        if !status.success() {
            return Err(OmmError::new(
                ErrorCode::Backend,
                false,
                "user service manager rejected operation; diagnostics withheld",
            ));
        }
        Ok(())
    }
}
pub fn installed(home: &Path) -> Result<Option<ServiceProfile>> {
    let file = plist(home);
    if !file.exists() {
        return Ok(None);
    }
    validate_private_file(&file)?;
    if std::fs::metadata(&file)?.len() > 64 * 1024 {
        return Err(OmmError::invalid("service profile exceeds limit"));
    }
    let bytes = std::fs::read(&file)?;
    if bytes.len() > 64 * 1024 {
        return Err(OmmError::invalid("service profile exceeds limit"));
    }
    decode_profile(&bytes).map(Some)
}
fn decode_profile(bytes: &[u8]) -> Result<ServiceProfile> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| OmmError::invalid("invalid service profile encoding"))?;
    let data = text
        .split("<!-- OMM_PROFILE:")
        .nth(1)
        .and_then(|s| s.split(" -->").next())
        .ok_or_else(|| OmmError::invalid("installed service profile missing"))?;
    let profile: ServiceProfile = serde_json::from_slice(
        &hex::decode(data).map_err(|_| OmmError::invalid("invalid installed service profile"))?,
    )?;
    if profile.schema_version != 1 {
        return Err(OmmError::invalid("unsupported service profile"));
    }
    Ok(profile)
}
pub async fn install(
    home: &Path,
    paths: &Paths,
    exe: &Path,
    runner: &dyn ServiceRunner,
) -> Result<()> {
    install_at(home, paths, exe, None, runner).await
}
pub async fn install_at(
    home: &Path,
    paths: &Paths,
    exe: &Path,
    runtime: Option<&crate::core::runtime::RuntimeLocation>,
    runner: &dyn ServiceRunner,
) -> Result<()> {
    validate_executable(exe)?;
    prepare_parent(home)?;
    // Shared by every installer/unregister operation for this HOME/label, independent of XDG.
    let directory = Directory::open(home)?;
    if let Some(current) = directory.snapshot(PLIST)? {
        let profile = decode_profile(&current.bytes)?;
        profile.check_roots(paths)?;
        if let Some(location) = runtime {
            profile.check_runtime(location)?;
        }
        let standard = render(home, exe, &profile, "Standard")?;
        if current.bytes == standard.as_bytes() {
            directory.verify(PLIST, Some(&current))?;
            return Ok(());
        }
        let legacy = render(home, exe, &profile, "Background")?;
        if current.bytes != legacy.as_bytes() {
            return Err(OmmError::new(ErrorCode::Conflict, false, "service registration differs from owned template; preserve it and review before changing registration"));
        }
        let location = runtime.ok_or_else(|| {
            OmmError::invalid("runtime location required to reload legacy registration")
        })?;
        return reload_legacy(
            home,
            paths,
            location,
            &directory,
            &current,
            standard.as_bytes(),
            runner,
        )
        .await;
    }
    let service_profile = runtime
        .map(|r| ServiceProfile::new_at(paths.clone(), r))
        .unwrap_or_else(|| ServiceProfile::new(paths.clone()));
    let body = render(home, exe, &service_profile, "Standard")?;
    ensure_sealed(paths, runtime, false)?;
    let published = directory.publish(PLIST, None, body.as_bytes())?;
    if let Err(e) = bootstrap(home, runner).await {
        if directory.remove(PLIST, &published).is_err() {
            return Err(OmmError::new(
                ErrorCode::Backend,
                false,
                "registration failed; changed metadata retained; review before recovery",
            ));
        }
        return Err(e);
    }
    directory.verify(PLIST, Some(&published))?;
    Ok(())
}
fn render(
    home: &Path,
    exe: &Path,
    service_profile: &ServiceProfile,
    process_type: &str,
) -> Result<String> {
    let paths = &service_profile.paths;
    let profile = hex::encode(serde_json::to_vec(service_profile)?);
    let mut env = String::new();
    for (key, path) in [
        ("XDG_CONFIG_HOME", &paths.config),
        ("XDG_DATA_HOME", &paths.data),
        ("XDG_STATE_HOME", &paths.state),
        ("XDG_CACHE_HOME", &paths.cache),
    ] {
        env.push_str(&format!(
            "<key>{key}</key><string>{}</string>",
            xml(path
                .parent()
                .unwrap()
                .to_str()
                .ok_or_else(|| OmmError::invalid("XDG roots must be UTF-8"))?)
        ));
    }
    if let Some(root) = &service_profile.explicit_runtime_root {
        env.push_str(&format!(
            "<key>XDG_RUNTIME_DIR</key><string>{}</string>",
            xml(root
                .to_str()
                .ok_or_else(|| OmmError::invalid("runtime root must be UTF-8"))?)
        ));
    }
    env.push_str(&format!(
        "<key>HOME</key><string>{}</string>",
        xml(home
            .to_str()
            .ok_or_else(|| OmmError::invalid("HOME must be UTF-8"))?)
    ));
    let body=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>{LABEL}</string><key>ProgramArguments</key><array><string>{}</string><string>service</string><string>run</string><string>--non-interactive</string></array><key>EnvironmentVariables</key><dict>{env}</dict><key>RunAtLoad</key><true/><key>KeepAlive</key><false/><key>ProcessType</key><string>{process_type}</string></dict></plist><!-- OMM_PROFILE:{profile} -->",xml(exe.to_str().ok_or_else(||OmmError::invalid("executable must be UTF-8"))?));
    Ok(body)
}
fn entry_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
fn validate_executable(exe: &Path) -> Result<()> {
    if !exe.is_absolute() || exe.canonicalize()? != exe {
        return Err(OmmError::invalid(
            "service executable must be absolute and canonical",
        ));
    }
    let meta = std::fs::symlink_metadata(exe)?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o022 != 0
        || meta.mode() & 0o100 == 0
    {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "service executable ownership, type or permissions unsafe",
        ));
    }
    Ok(())
}
fn prepare_parent(home: &Path) -> Result<()> {
    if !home.is_absolute() || home.canonicalize()? != home {
        return Err(OmmError::invalid(
            "service HOME must be absolute and canonical",
        ));
    }
    for dir in [
        home.to_path_buf(),
        home.join("Library"),
        home.join("Library/LaunchAgents"),
    ] {
        if !entry_exists(&dir)? {
            private_dir(&dir)?;
        }
        let meta = std::fs::symlink_metadata(&dir)?;
        if !meta.is_dir()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o022 != 0
            || dir.canonicalize()? != dir
        {
            return Err(OmmError::new(
                ErrorCode::Permission,
                false,
                "LaunchAgent directory unsafe",
            ));
        }
    }
    Ok(())
}
async fn bootstrap(home: &Path, runner: &dyn ServiceRunner) -> Result<()> {
    runner
        .run(&[
            "bootstrap".into(),
            format!("gui/{}", unsafe { libc::geteuid() }),
            plist(home).to_string_lossy().into_owned(),
        ])
        .await
}
async fn reload_legacy(
    home: &Path,
    paths: &Paths,
    location: &crate::core::runtime::RuntimeLocation,
    directory: &Directory,
    legacy: &Snapshot,
    standard: &[u8],
    runner: &dyn ServiceRunner,
) -> Result<()> {
    // Never unload a live service or an unsealed/adopted capture helper during registration.
    let guard = crate::core::runtime::RuntimeGuard::acquire(location, &paths.state)?;
    directory.verify(PLIST, Some(legacy))?;
    ensure_sealed(paths, Some(location), true)?;
    // Only exact owned templates are backed up; arbitrary environment values are never copied.
    match directory.snapshot(BACKUP)? {
        Some(old) if old.bytes == legacy.bytes => (),
        Some(_) => {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "registration backup differs; preserve it before upgrade",
            ))
        }
        None => {
            directory.publish(BACKUP, None, &legacy.bytes)?;
        }
    }
    runner
        .run(&[
            "bootout".into(),
            format!("gui/{}/{LABEL}", unsafe { libc::geteuid() }),
        ])
        .await?;
    let published = directory.publish(PLIST, Some(legacy), standard)?;
    // The registration lock stays held across activation/rollback; daemon startup needs only runtime.
    drop(guard);
    if bootstrap(home, runner).await.is_err() {
        let restored = directory.publish(PLIST, Some(&published), &legacy.bytes).map_err(|_| OmmError::new(ErrorCode::Backend, false, "registration reload failed; metadata changed or restore failed; backup retained; review before recovery"))?;
        let reactivated = bootstrap(home, runner).await.is_ok();
        if directory.verify(PLIST, Some(&restored)).is_err() {
            return Err(OmmError::new(
                ErrorCode::Backend,
                false,
                "registration rollback metadata changed; backup retained; review before recovery",
            ));
        }
        return Err(OmmError::new(
            ErrorCode::Backend,
            false,
            if reactivated {
                "registration reload failed; previous profile restored and re-registered; readiness unverified"
            } else {
                "registration reload failed; previous profile restored on disk; restart required; owned backup retained"
            },
        ));
    }
    directory.verify(PLIST, Some(&published))?;
    Ok(())
}
fn ensure_sealed(
    paths: &Paths,
    runtime: Option<&crate::core::runtime::RuntimeLocation>,
    require_capture_exit: bool,
) -> Result<()> {
    if entry_exists(&paths.state.join("state.sqlite"))? {
        let store = crate::core::store::Store::open_read_only(paths)?;
        if store.has_unsealed_segmented_sessions()?
            || (require_capture_exit
                && (store.state::<serde_json::Value>("recording")?.is_some()
                    || store
                        .state::<bool>("capture_helper_expected")?
                        .unwrap_or(false)))
        {
            return Err(OmmError::new(ErrorCode::Conflict, false, "stop service and confirm recording seal and capture exit before registration upgrade"));
        }
    }
    if require_capture_exit
        && runtime
            .map(|r| entry_exists(&r.path.join("capture/audio_daemon.sock")))
            .transpose()?
            .unwrap_or(false)
    {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "confirm capture exit before registration upgrade",
        ));
    }
    Ok(())
}
pub async fn start(home: &Path, runner: &dyn ServiceRunner) -> Result<()> {
    prepare_parent(home)?;
    let directory = Directory::open(home)?;
    let current = directory
        .snapshot(PLIST)?
        .ok_or_else(|| OmmError::unavailable("service not installed"))?;
    decode_profile(&current.bytes)?;
    runner
        .run(&[
            "kickstart".into(),
            format!("gui/{}/{LABEL}", unsafe { libc::geteuid() }),
        ])
        .await?;
    directory.verify(PLIST, Some(&current))
}
pub async fn unregister(home: &Path, runner: &dyn ServiceRunner) -> Result<()> {
    prepare_parent(home)?;
    let directory = Directory::open(home)?;
    unregister_locked(&directory, runner).await
}
async fn unregister_locked(directory: &Directory, runner: &dyn ServiceRunner) -> Result<()> {
    let current = directory
        .snapshot(PLIST)?
        .ok_or_else(|| OmmError::unavailable("service not installed"))?;
    decode_profile(&current.bytes)?;
    runner
        .run(&[
            "bootout".into(),
            format!("gui/{}/{LABEL}", unsafe { libc::geteuid() }),
        ])
        .await?;
    directory.remove(PLIST, &current)
}

/// Shared CLI uninstall path; the injected runner keeps tests off real LaunchAgents.
/// A shutdown conflict from a live service (including capture identity rejection)
/// must never be mistaken for an absent service and followed by bootout.
pub async fn uninstall_at(
    home: &Path,
    paths: &Paths,
    runtime: &crate::core::runtime::RuntimeLocation,
    runner: &dyn ServiceRunner,
) -> Result<serde_json::Value> {
    prepare_parent(home)?;
    let directory = Directory::open(home)?;
    let mut inactive_guard = None;
    let stopped = match super::daemon::request_at(paths, runtime, "shutdown", None).await {
        Ok(value) => Some(value),
        Err(error) if error.code == ErrorCode::Conflict => {
            inactive_guard = Some(crate::core::runtime::RuntimeGuard::acquire(
                runtime,
                &paths.state,
            )?);
            let store = crate::core::store::Store::open(paths)?;
            let endpoint = runtime.path.join("capture/audio_daemon.sock");
            let endpoint_present = match std::fs::symlink_metadata(endpoint) {
                Ok(_) => true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => return Err(e.into()),
            };
            if store
                .state::<super::daemon::Recording>("recording")?
                .is_some()
                || store
                    .state::<bool>("capture_helper_expected")?
                    .unwrap_or(false)
                || endpoint_present
            {
                return Err(OmmError::new(
                    ErrorCode::Conflict,
                    false,
                    "restart service and confirm recording seal and capture exit before uninstall",
                ));
            }
            None
        }
        Err(error) => return Err(error),
    };
    unregister_locked(&directory, runner).await?;
    drop(inactive_guard);
    Ok(serde_json::json!({"unregistered":true,"dataPreserved":true,"sealed":stopped}))
}

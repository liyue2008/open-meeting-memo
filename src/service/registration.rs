use super::ServiceProfile;
use crate::core::{contracts::*, paths::*};
use async_trait::async_trait;
use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
const LABEL: &str = "org.openmeetingmemo.service";
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
}
pub struct Launchctl;
#[async_trait]
impl ServiceRunner for Launchctl {
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
    let bytes = std::fs::read(&file)?;
    if bytes.len() > 64 * 1024 {
        return Err(OmmError::invalid("service profile exceeds limit"));
    }
    let text = std::str::from_utf8(&bytes)
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
    Ok(Some(profile))
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
    if !exe.is_absolute() || !exe.is_file() {
        return Err(OmmError::invalid("service executable must be absolute"));
    }
    if installed(home)?.is_some() {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "service already installed; uninstall before explicitly changing roots",
        ));
    }
    let parent = plist(home).parent().unwrap().to_path_buf();
    if !parent.exists() {
        private_dir(&parent)?;
    }
    let meta = std::fs::symlink_metadata(&parent)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "LaunchAgent directory unsafe",
        ));
    }
    let service_profile = runtime
        .map(|r| ServiceProfile::new_at(paths.clone(), r))
        .unwrap_or_else(|| ServiceProfile::new(paths.clone()));
    let profile = hex::encode(serde_json::to_vec(&service_profile)?);
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
    let body=format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Label</key><string>{LABEL}</string><key>ProgramArguments</key><array><string>{}</string><string>service</string><string>run</string><string>--non-interactive</string></array><key>EnvironmentVariables</key><dict>{env}</dict><key>RunAtLoad</key><true/><key>KeepAlive</key><false/><key>ProcessType</key><string>Background</string></dict></plist><!-- OMM_PROFILE:{profile} -->",xml(exe.to_str().ok_or_else(||OmmError::invalid("executable must be UTF-8"))?));
    write_private_new(&plist(home), body.as_bytes())?;
    if let Err(e) = runner
        .run(&[
            "bootstrap".into(),
            format!("gui/{}", unsafe { libc::geteuid() }),
            plist(home).to_string_lossy().into_owned(),
        ])
        .await
    {
        let _ = std::fs::remove_file(plist(home));
        return Err(e);
    }
    Ok(())
}
pub async fn start(home: &Path, runner: &dyn ServiceRunner) -> Result<()> {
    if installed(home)?.is_none() {
        return Err(OmmError::unavailable("service not installed"));
    }
    runner
        .run(&[
            "kickstart".into(),
            format!("gui/{}/{LABEL}", unsafe { libc::geteuid() }),
        ])
        .await
}
pub async fn unregister(home: &Path, runner: &dyn ServiceRunner) -> Result<()> {
    if installed(home)?.is_none() {
        return Err(OmmError::unavailable("service not installed"));
    }
    runner
        .run(&[
            "bootout".into(),
            format!("gui/{}/{LABEL}", unsafe { libc::geteuid() }),
        ])
        .await?;
    std::fs::remove_file(plist(home))?;
    sync_dir(plist(home).parent().unwrap())
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
    unregister(home, runner).await?;
    drop(inactive_guard);
    Ok(serde_json::json!({"unregistered":true,"dataPreserved":true,"sealed":stopped}))
}

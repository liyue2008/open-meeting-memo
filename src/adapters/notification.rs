//! macOS system script-host notifications, not an independently authorized OMM app.
use crate::core::{
    contracts::*,
    notifications::{NotificationAdapter, NotificationEvent},
    process::{OwnedChild, Reapers},
};
use async_trait::async_trait;
use std::{path::PathBuf, process::Stdio};

// A constant script plus a typed, static argv message: never interpolate user text into AppleScript.
const SCRIPT: &str =
    "on run argv\n display notification (item 1 of argv) with title \"Open Meeting Memo\"\nend run";
#[derive(Default)]
pub struct MacNotificationAdapter {
    reapers: Reapers,
}
impl MacNotificationAdapter {
    pub fn available() -> bool {
        cfg!(target_os = "macos") && std::path::Path::new("/usr/bin/osascript").is_file()
    }
    pub fn notification_command(event: NotificationEvent) -> tokio::process::Command {
        let mut command = Self::isolated_command(PathBuf::from("/usr/bin/osascript"));
        command.args(["-e", SCRIPT, "--", event.message()]);
        command
    }
    fn isolated_command(executable: PathBuf) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(executable);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
    pub async fn request_settings(&self) -> Result<()> {
        if !Self::available() {
            return Err(OmmError::unavailable(
                "macOS notification channel unavailable",
            ));
        }
        let mut command = Self::isolated_command(PathBuf::from("/usr/bin/open"));
        command.arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension");
        self.call(command).await
    }
    async fn call(&self, mut command: tokio::process::Command) -> Result<()> {
        let mut child = OwnedChild::new(
            command
                .spawn()
                .map_err(|_| OmmError::unavailable("notification process unavailable"))?,
            self.reapers.clone(),
            None,
        );
        if !child.wait().await?.success() {
            return Err(OmmError::unavailable("notification call rejected"));
        }
        Ok(())
    }
}
#[async_trait]
impl NotificationAdapter for MacNotificationAdapter {
    async fn send(&self, event: NotificationEvent) -> Result<()> {
        if !Self::available() {
            return Err(OmmError::unavailable(
                "macOS notification channel unavailable",
            ));
        }
        self.call(Self::notification_command(event)).await
    }
    async fn drain(&self) -> Result<()> {
        let cleanup = self.reapers.drain();
        tokio::pin!(cleanup);
        if let Ok(result) =
            tokio::time::timeout(std::time::Duration::from_secs(3), &mut cleanup).await
        {
            return result;
        }
        // Keep ownership until exit is confirmed. A monitoring budget is not a license
        // to abandon a live child or claim a strictly bounded service shutdown.
        eprintln!("warning: notification child cleanup still pending; waiting for owned exit");
        cleanup.await
    }
}

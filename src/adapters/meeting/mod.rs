use crate::core::contracts::*;

/// Parse bounded observations supplied by the Lark edge; unknown evidence cannot end recording.
/// The live collector combines participant identity, local room logs and weak process/window evidence.
pub fn classify(
    observed_at_ms: i64,
    process_present: Option<bool>,
    window_title: Option<&str>,
    api_participant_status: Option<u32>,
    api_room_id: Option<&str>,
    fresh_local_room_id: Option<&str>,
) -> MeetingObservation {
    let strong = api_participant_status == Some(2)
        && api_room_id.is_some()
        && api_room_id == fresh_local_room_id;
    let (state, evidence) = if strong {
        (MeetingState::Active, Evidence::Strong)
    } else if process_present == Some(false) && window_title.is_none() {
        (MeetingState::Inactive, Evidence::Strong)
    } else if process_present == Some(true) || window_title.is_some() {
        (MeetingState::Active, Evidence::Weak)
    } else {
        (MeetingState::Unknown, Evidence::None)
    };
    MeetingObservation {
        state,
        evidence,
        observed_at_ms,
        source_id: if strong {
            api_room_id.map(str::to_owned)
        } else {
            None
        },
        title: window_title.map(str::to_owned),
    }
}

use crate::core::{
    config::AppConfig,
    process::{OwnedChild, Reapers},
};

const OBSERVATION_BUDGET: std::time::Duration = std::time::Duration::from_secs(8);
const MAX_PARTICIPANT_QUERIES: usize = 8;
use async_trait::async_trait;
use serde_json::Value;
use std::{
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{io::AsyncReadExt, process::Command};

pub struct LarkAdapter {
    config: AppConfig,
    capture: Arc<dyn CaptureAdapter>,
    reapers: Reapers,
}
impl LarkAdapter {
    pub fn new(config: AppConfig, capture: Arc<dyn CaptureAdapter>) -> Self {
        Self::with_reapers(config, capture, Reapers::default())
    }
    pub fn with_reapers(
        config: AppConfig,
        capture: Arc<dyn CaptureAdapter>,
        reapers: Reapers,
    ) -> Self {
        Self {
            config,
            capture,
            reapers,
        }
    }
}
async fn command_json(exe: &Path, args: &[&str], reapers: &Reapers) -> Result<Value> {
    if !exe.is_absolute() {
        return Err(OmmError::unavailable("Lark CLI not configured"));
    }
    let parent = exe
        .parent()
        .ok_or_else(|| OmmError::invalid("invalid Lark executable"))?;
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .env("PATH", format!("{}:/usr/bin:/bin", parent.display()))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    for key in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        cmd.env_remove(key);
    }
    let mut child = OwnedChild::new(
        cmd.spawn()
            .map_err(|_| OmmError::unavailable("Lark observer executable unavailable"))?,
        reapers.clone(),
        None,
    );
    let stdout = child.stdout.take().unwrap();
    let operation = async {
        let mut bytes = vec![];
        stdout.take(1024 * 1024 + 1).read_to_end(&mut bytes).await?;
        if bytes.len() > 1024 * 1024 {
            return Err(OmmError::new(
                ErrorCode::Backend,
                false,
                "Lark response exceeds observation limit",
            ));
        }
        if !child.wait().await?.success() {
            return Err(OmmError::unavailable(
                "Lark API observation unavailable; check user authentication and API permissions",
            ));
        }
        Ok(serde_json::from_slice(&bytes)?)
    };
    match tokio::time::timeout(std::time::Duration::from_secs(8), operation).await {
        Ok(r) => r,
        Err(_) => {
            let _ = child.kill().await;
            Err(OmmError::new(
                ErrorCode::Timeout,
                true,
                "Lark observation timed out",
            ))
        }
    }
}
pub fn rtc_room_matches(dirs: &[PathBuf], room: &str, now: std::time::SystemTime) -> bool {
    let mut files = vec![];
    for dir in dirs.iter().take(8) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.take(4096).flatten() {
                let p = entry.path();
                if !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("_rtclog.log"))
                {
                    continue;
                }
                if let Ok(m) = std::fs::symlink_metadata(&p) {
                    if !m.is_file() {
                        continue;
                    }
                    if let Ok(time) = m.modified() {
                        let fresh = now
                            .duration_since(time)
                            .map(|d| d.as_secs() <= 15)
                            .unwrap_or_else(|_| {
                                time.duration_since(now).is_ok_and(|d| d.as_secs() <= 5)
                            });
                        if fresh {
                            files.push((time, m.len(), p));
                        }
                    }
                }
            }
        }
    }
    files.sort_by_key(|row| std::cmp::Reverse(row.0));
    files.into_iter().take(3).any(|(_, size, path)| {
        let Ok(mut file) = std::fs::File::open(path) else {
            return false;
        };
        let limit = 4 * 1024 * 1024;
        if file
            .seek(std::io::SeekFrom::Start(size.saturating_sub(limit)))
            .is_err()
        {
            return false;
        }
        let mut bytes = vec![];
        if file.take(limit).read_to_end(&mut bytes).is_err() {
            return false;
        }
        let pattern = format!(r"room_id[:_]{}(?:[^A-Za-z0-9_-]|$)", regex::escape(room));
        regex::Regex::new(&pattern).is_ok_and(|re| re.is_match(&String::from_utf8_lossy(&bytes)))
    })
}
impl LarkAdapter {
    async fn observe_inner(&self) -> Result<MeetingObservation> {
        let now = chrono::Utc::now().timestamp_millis();
        let windows = self.capture.windows().await.unwrap_or_default();
        let title = windows
            .iter()
            .find(|w| {
                ["lark", "feishu", "飞书"]
                    .iter()
                    .any(|p| w.app.to_lowercase().contains(p))
                    && ["Meeting", "会议", "通话"]
                        .iter()
                        .any(|p| w.title.contains(p))
            })
            .map(|w| w.title.clone());
        if !self.config.lark_executable.as_os_str().is_empty() {
            let result: Result<MeetingObservation> = async {
                let auth = command_json(
                    &self.config.lark_executable,
                    &["auth", "status"],
                    &self.reapers,
                )
                .await?;
                let user = auth["identities"]["user"]["openId"]
                    .as_str()
                    .ok_or_else(|| OmmError::unavailable("Lark user authentication unavailable"))?;
                let active = command_json(
                    &self.config.lark_executable,
                    &[
                        "vc",
                        "+meeting-list-active",
                        "--as",
                        "user",
                        "--format",
                        "json",
                    ],
                    &self.reapers,
                )
                .await?;
                let meetings = active["data"]["meetings"].as_array().ok_or_else(|| {
                    OmmError::new(
                        ErrorCode::Corrupt,
                        false,
                        "Lark active meeting schema invalid",
                    )
                })?;
                for m in meetings.iter().take(MAX_PARTICIPANT_QUERIES) {
                    let Some(id) = m["meeting_id"].as_str().filter(|id| {
                        !id.is_empty()
                            && id.len() <= 128
                            && id
                                .chars()
                                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
                    }) else {
                        continue;
                    };
                    let participants = command_json(
                        &self.config.lark_executable,
                        &[
                            "vc",
                            "meeting",
                            "get",
                            "--as",
                            "user",
                            "--meeting-id",
                            id,
                            "--with-participants",
                            "--user-id-type",
                            "open_id",
                            "--format",
                            "json",
                        ],
                        &self.reapers,
                    )
                    .await?;
                    let joined = participants["data"]["meeting"]["participants"]
                        .as_array()
                        .is_some_and(|ps| {
                            ps.iter().any(|p| {
                                p["id"].as_str() == Some(user)
                                    && (p["status"].as_u64() == Some(2)
                                        || p["status"].as_str() == Some("2"))
                            })
                        });
                    if joined
                        && rtc_room_matches(
                            &self.config.rtc_log_dirs,
                            id,
                            std::time::SystemTime::now(),
                        )
                    {
                        let mut observation = classify(
                            now,
                            Some(true),
                            title.as_deref(),
                            Some(2),
                            Some(id),
                            Some(id),
                        );
                        observation.title = m["meeting_title"]
                            .as_str()
                            .map(str::to_owned)
                            .or(title.clone());
                        return Ok(observation);
                    }
                    if joined {
                        // API says the user is joined but the local room is unconfirmed.
                        // Missing/stale logs cannot be treated as the meeting ending.
                        return Ok(classify(
                            now,
                            Some(true),
                            title.as_deref(),
                            None,
                            None,
                            None,
                        ));
                    }
                }
                if meetings.len() > MAX_PARTICIPANT_QUERIES {
                    return Err(OmmError::unavailable(
                        "Lark participant query budget exhausted",
                    ));
                }
                // An empty active list plus no local meeting process is an explicit negative; failures never are.
                let present = meeting_process_present(&self.reapers).await;
                Ok(classify(now, present, title.as_deref(), None, None, None))
            }
            .await;
            return result;
        }
        let present = meeting_process_present(&self.reapers).await;
        Ok(classify(now, present, title.as_deref(), None, None, None))
    }
}
async fn meeting_process_present(reapers: &Reapers) -> Option<bool> {
    let child = Command::new("/bin/ps")
        .args(["-axo", "command="])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut child = OwnedChild::new(child, reapers.clone(), None);
    let stdout = child.stdout.take()?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut bytes = vec![];
        stdout
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .ok()?;
        if bytes.len() > 1024 * 1024 || !child.wait().await.ok()?.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&bytes).lines().any(|line| {
            ["LarkSuite.app", "/Lark", "Feishu"]
                .iter()
                .any(|s| line.contains(s))
                && [
                    "byteview",
                    "--scene=vc",
                    "--scene=meeting",
                    "--module=renderer-byteview",
                    "--module=renderer-meeting",
                ]
                .iter()
                .any(|s| line.contains(s))
        }))
    })
    .await;
    match result {
        Ok(p) => p,
        Err(_) => {
            let _ = child.kill().await;
            None
        }
    }
}

#[async_trait]
impl MeetingAdapter for LarkAdapter {
    async fn observe(&self) -> Result<MeetingObservation> {
        let result = tokio::time::timeout(OBSERVATION_BUDGET, self.observe_inner()).await;
        self.reapers.drain().await?;
        match result {
            Ok(Ok(observation)) => Ok(observation),
            _ => Ok(MeetingObservation {
                state: MeetingState::Unknown,
                evidence: Evidence::None,
                source_id: None,
                title: None,
                observed_at_ms: chrono::Utc::now().timestamp_millis(),
            }),
        }
    }
}

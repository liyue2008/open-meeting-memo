use super::{contracts::*, paths::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    #[serde(skip)]
    #[doc(hidden)]
    pub loaded_revision: Option<String>,
    pub audio: AudioConfig,
    pub version: String,
    pub notifications: super::notifications::NotificationConfig,
    pub asr: AsrConfig,
    pub pi: PiConfig,
    pub capture_executable: PathBuf,
    pub worker_executable: PathBuf,
    pub lark_executable: PathBuf,
    pub rtc_log_dirs: Vec<PathBuf>,
    pub auto_record: bool,
    pub recording_consent: bool,
    pub start_delay_ms: i64,
    pub end_grace_ms: i64,
    pub poll_ms: u64,
    pub segment_ms: u64,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            loaded_revision: None,
            version: "1".into(),
            audio: AudioConfig::default(),
            notifications: Default::default(),
            asr: AsrConfig {
                threads: 3,
                timeout_ms: 7_200_000,
                model_version: "sherpa-onnx-1.13.2-fire-red-asr-large-zh_en-2025-02-16-int8".into(),
                ..Default::default()
            },
            pi: PiConfig {
                prompt_version: "minutes-v1".into(),
                timeout_ms: 300_000,
                max_input_bytes: 128 * 1024,
                max_output_bytes: 128 * 1024,
                ..Default::default()
            },
            capture_executable: PathBuf::new(),
            worker_executable: PathBuf::new(),
            lark_executable: PathBuf::new(),
            rtc_log_dirs: vec![],
            auto_record: false,
            recording_consent: false,
            start_delay_ms: 15_000,
            end_grace_ms: 30_000,
            poll_ms: 2000,
            segment_ms: 1_800_000,
        }
    }
}
impl AppConfig {
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.config.join("config.json");
        if !file.is_file() {
            return Err(OmmError::unavailable("configuration not initialized"));
        }
        validate_private_file(&file)?;
        let bytes = fs::read(file)?;
        let mut config: Self = serde_json::from_slice(&bytes)?;
        config.loaded_revision = Some(super::store::hash_bytes(&bytes));
        if config.segment_ms == 3_600_000 {
            eprintln!("warning: legacy segment_ms=3600000 clamped in memory to 1800000; configuration file unchanged");
            config.segment_ms = 1_800_000;
        }
        Ok(config)
    }
    pub fn save(&self, paths: &Paths) -> Result<()> {
        let transaction = super::config_transaction::ConfigTransaction::acquire(paths)?;
        transaction.compare(&self.loaded_revision)?;
        transaction.publish(paths, &serde_json::to_vec_pretty(self)?)
    }
    pub fn snapshot(&self) -> ConfigSnapshot {
        let mut asr = self.asr.clone();
        asr.threads = asr
            .threads
            .min(super::resources::CpuBudget::current().asr_threads)
            .max(1);
        ConfigSnapshot {
            audio: self.audio.clone(),
            version: self.version.clone(),
            asr,
            pi: self.pi.clone(),
        }
    }
    pub fn validate_policy(&self) -> Result<()> {
        self.audio.validate()?;
        if self.start_delay_ms < 0
            || self.end_grace_ms < 0
            || self.poll_ms < 100
            || self.poll_ms > 60_000
            || !(60_000..=1_800_000).contains(&self.segment_ms)
            || (self.auto_record && !self.recording_consent)
        {
            return Err(OmmError::invalid(
                "invalid recording timing or consent policy",
            ));
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_policy()?;
        if self.version.is_empty()
            || !self.capture_executable.is_absolute()
            || !self.worker_executable.is_absolute()
            || self.asr.model_version.is_empty()
            || self.asr.threads == 0
            || self.asr.timeout_ms == 0
            || self.pi.provider.is_empty()
            || self.pi.model.is_empty()
            || self.pi.prompt_version.is_empty()
            || !self.pi.executable.is_absolute()
            || self.pi.timeout_ms == 0
            || self.pi.max_input_bytes < 1024
            || self.pi.max_output_bytes == 0
            || [
                &self.asr.library_path,
                &self.asr.encoder_path,
                &self.asr.decoder_path,
                &self.asr.tokens_path,
                &self.asr.vad_path,
            ]
            .iter()
            .any(|p| !p.is_absolute())
        {
            return Err(OmmError::invalid("configuration is incomplete"));
        }
        if !self.lark_executable.as_os_str().is_empty() && !self.lark_executable.is_absolute()
            || self.rtc_log_dirs.iter().any(|p| !p.is_absolute())
        {
            return Err(OmmError::invalid("Lark paths must be absolute"));
        }
        Ok(())
    }
    /// Only typed product fields are editable; never store credentials or grant recording consent here.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let allowed = [
            "audio.opus_bitrate_kbps",
            "notifications.enabled",
            "notifications.show_meeting_title",
            "notifications.events",
            "capture_executable",
            "worker_executable",
            "lark_executable",
            "rtc_log_dirs",
            "start_delay_ms",
            "end_grace_ms",
            "poll_ms",
            "segment_ms",
            "asr.library_path",
            "asr.encoder_path",
            "asr.decoder_path",
            "asr.tokens_path",
            "asr.vad_path",
            "asr.threads",
            "asr.timeout_ms",
            "pi.executable",
            "pi.node_executable",
            "pi.provider",
            "pi.model",
            "pi.prompt_version",
            "pi.timeout_ms",
            "pi.max_input_bytes",
            "pi.max_output_bytes",
        ];
        if !allowed.contains(&key) {
            return Err(OmmError::invalid(
                "unsupported config key; credentials and consent cannot be set here",
            ));
        }
        let mut doc = serde_json::to_value(&*self)?;
        let mut keys = key.split('.');
        let first = keys.next().unwrap();
        let slot = if let Some(second) = keys.next() {
            &mut doc[first][second]
        } else {
            &mut doc[first]
        };
        *slot = match slot {
            _ if key == "pi.node_executable" => {
                if value == "null" {
                    Value::Null
                } else {
                    Value::String(value.into())
                }
            }
            Value::String(_) => Value::String(value.into()),
            _ => serde_json::from_str(value)
                .map_err(|_| OmmError::invalid("config value has invalid type"))?,
        };
        let mut next: Self = serde_json::from_value(doc)
            .map_err(|_| OmmError::invalid("config value has invalid type"))?;
        next.loaded_revision = self.loaded_revision.clone();
        next.validate_policy()?;
        next.version = uuid::Uuid::new_v4().to_string();
        *self = next;
        Ok(())
    }
}

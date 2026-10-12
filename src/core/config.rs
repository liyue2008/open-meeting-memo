use super::{contracts::*, paths::*};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub version: String,
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
            version: "1".into(),
            asr: AsrConfig {
                threads: 2,
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
            segment_ms: 3_600_000,
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
        Ok(serde_json::from_slice(&fs::read(file)?)?)
    }
    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_private_replace(
            &paths.config.join("config.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
    pub fn snapshot(&self) -> ConfigSnapshot {
        ConfigSnapshot {
            version: self.version.clone(),
            asr: self.asr.clone(),
            pi: self.pi.clone(),
        }
    }
    pub fn validate_policy(&self) -> Result<()> {
        if self.start_delay_ms < 0
            || self.end_grace_ms < 0
            || self.poll_ms < 100
            || self.poll_ms > 60_000
            || !(60_000..=3_600_000).contains(&self.segment_ms)
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
            Value::String(_) => Value::String(value.into()),
            _ => serde_json::from_str(value)
                .map_err(|_| OmmError::invalid("config value has invalid type"))?,
        };
        let mut next: Self = serde_json::from_value(doc)
            .map_err(|_| OmmError::invalid("config value has invalid type"))?;
        next.validate_policy()?;
        next.version = uuid::Uuid::new_v4().to_string();
        *self = next;
        Ok(())
    }
}

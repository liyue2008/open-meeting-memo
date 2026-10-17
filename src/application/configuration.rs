use crate::core::{
    config::AppConfig, config_transaction::ConfigTransaction, contracts::*, paths::Paths,
};
use serde_json::{json, Value};
const FIELDS: &[(&str, &str)] = &[
    ("opusBitrateKbps", "audio.opus_bitrate_kbps"),
    ("notificationsEnabled", "notifications.enabled"),
    (
        "notificationsShowMeetingTitle",
        "notifications.show_meeting_title",
    ),
    ("notificationEvents", "notifications.events"),
    ("captureExecutable", "capture_executable"),
    ("workerExecutable", "worker_executable"),
    ("larkExecutable", "lark_executable"),
    ("rtcLogDirs", "rtc_log_dirs"),
    ("startDelayMs", "start_delay_ms"),
    ("endGraceMs", "end_grace_ms"),
    ("pollMs", "poll_ms"),
    ("segmentMs", "segment_ms"),
    ("asrLibraryPath", "asr.library_path"),
    ("asrEncoderPath", "asr.encoder_path"),
    ("asrDecoderPath", "asr.decoder_path"),
    ("asrTokensPath", "asr.tokens_path"),
    ("asrVadPath", "asr.vad_path"),
    ("asrThreads", "asr.threads"),
    ("asrTimeoutMs", "asr.timeout_ms"),
    ("piExecutable", "pi.executable"),
    ("piNodeExecutable", "pi.node_executable"),
    ("piProvider", "pi.provider"),
    ("piModel", "pi.model"),
    ("piPromptVersion", "pi.prompt_version"),
    ("piTimeoutMs", "pi.timeout_ms"),
    ("piMaxInputBytes", "pi.max_input_bytes"),
    ("piMaxOutputBytes", "pi.max_output_bytes"),
];
fn pointer(key: &str) -> String {
    format!("/{}", key.replace('.', "/"))
}
fn current(paths: &Paths) -> Result<AppConfig> {
    if paths.config.join("config.json").try_exists()? {
        AppConfig::load(paths)
    } else {
        Ok(AppConfig::default())
    }
}
pub fn apply(config: &AppConfig, patch: &Value) -> Result<AppConfig> {
    super::validation::validate_named("ConfigPatch", patch)?;
    let mut document = serde_json::to_value(config)?;
    for (name, value) in patch
        .as_object()
        .ok_or_else(|| OmmError::invalid("invalid configuration patch"))?
    {
        let key = FIELDS
            .iter()
            .find_map(|(field, key)| (*field == name).then_some(*key))
            .ok_or_else(|| OmmError::invalid("unknown configuration field"))?;
        *document
            .pointer_mut(&pointer(key))
            .ok_or_else(|| OmmError::invalid("configuration field is unavailable"))? =
            value.clone();
    }
    let mut next: AppConfig = serde_json::from_value(document)?;
    next.loaded_revision = config.loaded_revision.clone();
    next.validate_policy()?;
    if !(1..=3).contains(&next.asr.threads)
        || next.asr.timeout_ms == 0
        || next.pi.timeout_ms == 0
        || next.pi.max_input_bytes < 1024
        || next.pi.max_output_bytes == 0
    {
        return Err(OmmError::invalid("invalid processing budget"));
    }
    next.version = uuid::Uuid::new_v4().to_string();
    Ok(next)
}
pub fn validation(paths: &Paths, patch: &Value) -> Result<Value> {
    let cfg = current(paths)?;
    let result = apply(&cfg, patch);
    let complete = result.as_ref().is_ok_and(|cfg| cfg.validate().is_ok());
    Ok(
        json!({"valid":result.is_ok(),"fieldErrors":if result.is_ok(){vec![]}else{vec![json!({"field":"configuration","message":"Invalid timing, path, or processing budget"})]},"effects":patch.as_object().into_iter().flat_map(|fields|fields.keys()).map(|field|json!({"field":field,"takesEffect":if field=="opusBitrateKbps"{"new_segment"}else if field.starts_with("asr")||field.starts_with("pi") {"new_task"}else if field=="captureExecutable"||field=="workerExecutable"{"service_restart"}else{"service_reload"}})).collect::<Vec<_>>(),"structurallyComplete":complete}),
    )
}
pub fn snapshot(paths: &Paths, compatibility: &Value) -> Result<Value> {
    let initialized = paths.config.join("config.json").try_exists()?;
    let config = current(paths)?;
    let document = serde_json::to_value(&config)?;
    let mut values = serde_json::Map::new();
    for (name, key) in FIELDS {
        values.insert(
            (*name).into(),
            document
                .pointer(&pointer(key))
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    let runtime = paths.runtime_location(&std::env::vars().collect()).ok();
    Ok(
        json!({"revision":config.loaded_revision,"values":values,"initialized":initialized,"structurallyComplete":config.validate().is_ok(),"modelVersion":config.asr.model_version,"roots":{"config":paths.config,"data":paths.data,"state":paths.state,"cache":paths.cache,"runtime":runtime.map(|r|r.path.to_string_lossy().into_owned()).unwrap_or_default()},"autoRecord":config.auto_record,"recordingConsent":config.recording_consent,"warnings":if initialized{vec![]}else{vec!["Configuration is not initialized; displayed defaults have not been saved"]},"compatibility":compatibility}),
    )
}
pub fn save(paths: &Paths, expected: &Option<String>, patch: &Value) -> Result<()> {
    let transaction = ConfigTransaction::acquire(paths)?;
    transaction.compare(expected)?;
    let config = current(paths)?;
    let next = apply(&config, patch)?;
    transaction.publish(paths, &serde_json::to_vec_pretty(&next)?)
}
pub fn auto_set(paths: &Paths, enabled: bool, consent: bool) -> Result<()> {
    if enabled && !consent {
        return Err(OmmError::new(
            ErrorCode::ConsentRequired,
            false,
            "recording consent must be confirmed",
        ));
    }
    let transaction = ConfigTransaction::acquire(paths)?;
    let mut cfg: AppConfig = transaction
        .bytes
        .as_deref()
        .map(serde_json::from_slice)
        .transpose()?
        .ok_or_else(|| OmmError::unavailable("configuration not initialized"))?;
    cfg.auto_record = enabled;
    if enabled {
        cfg.recording_consent = true;
    }
    cfg.version = uuid::Uuid::new_v4().to_string();
    cfg.validate_policy()?;
    transaction.publish(paths, &serde_json::to_vec_pretty(&cfg)?)
}

pub fn cli_field(key: &str, config: &AppConfig) -> Result<(String, Value)> {
    let field = FIELDS
        .iter()
        .find(|(_, candidate)| *candidate == key)
        .ok_or_else(|| OmmError::invalid("unsupported config key"))?
        .0;
    let document = serde_json::to_value(config)?;
    Ok((
        field.into(),
        document
            .pointer(&pointer(key))
            .ok_or_else(|| OmmError::invalid("missing configuration field"))?
            .clone(),
    ))
}

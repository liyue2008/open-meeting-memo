//! Cross-file writes use a recoverable journal; no SQLite transaction claims to roll back JSON.
use super::{configuration, profile::ProfileOwner};
use crate::core::{
    config::AppConfig, config_transaction::ConfigTransaction, contracts::*, paths::*,
    store::hash_bytes,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    generation: uuid::Uuid,
    body_hash: String,
    expected: Option<String>,
    target: AppConfig,
    target_hash: String,
    result: Option<Value>,
}
pub fn save(paths: &Paths, owner: &ProfileOwner, command: &str, request: &Value) -> Result<Value> {
    let lock = ConfigTransaction::acquire(paths)?;
    let mut body = request.clone();
    body["payload"]
        .as_object_mut()
        .ok_or_else(|| OmmError::invalid("missing payload"))?
        .remove("admission");
    let body_hash = hash_bytes(&serde_json::to_vec(&json!([command, body]))?);
    let id = hash_bytes(
        request["requestId"]
            .as_str()
            .ok_or_else(|| OmmError::invalid("missing request ID"))?
            .as_bytes(),
    );
    let root = paths.config.join("desktop-journals");
    private_dir(&root)?;
    let path = root.join(format!("{id}.json"));
    let mut journal: Journal = if path.try_exists()? {
        validate_private_file(&path)?;
        if std::fs::metadata(&path)?.len() > 256 * 1024 {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "configuration journal exceeds budget",
            ));
        }
        serde_json::from_slice(&std::fs::read(&path)?)?
    } else {
        // Use the same supported legacy normalization as every other config writer.
        // The writer lock pins these bytes; raw-hash CAS still compares the original file.
        let cfg = if lock.bytes.is_some() {
            AppConfig::load(paths)?
        } else {
            AppConfig::default()
        };
        let target = if command == "config_save" {
            let expected = serde_json::from_value(request["payload"]["expectedRevision"].clone())?;
            lock.compare(&expected)?;
            configuration::apply(&cfg, &request["payload"]["patch"])?
        } else {
            let mut cfg = cfg;
            let enabled = request["payload"]["enabled"]
                .as_bool()
                .ok_or_else(|| OmmError::invalid("invalid auto recording preference"))?;
            if enabled && request["payload"]["confirmRecordingConsent"] != true {
                return Err(OmmError::new(
                    ErrorCode::ConsentRequired,
                    false,
                    "recording consent must be confirmed",
                ));
            }
            cfg.auto_record = enabled;
            if enabled {
                cfg.recording_consent = true;
            }
            cfg.version = uuid::Uuid::new_v4().to_string();
            cfg.validate_policy()?;
            cfg
        };
        // AppConfig's deny_unknown_fields parser excludes authentication/provider extension files.
        let target_bytes = serde_json::to_vec_pretty(&target)?;
        let journal = Journal {
            generation: owner.generation,
            body_hash: body_hash.clone(),
            expected: lock.revision.clone(),
            target,
            target_hash: hash_bytes(&target_bytes),
            result: None,
        };
        write_private_new(&path, &serde_json::to_vec(&journal)?)?;
        journal
    };
    if journal.generation != owner.generation || journal.body_hash != body_hash {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "configuration request belongs to another generation or payload",
        ));
    }
    if let Some(result) = journal.result {
        return Ok(result);
    }
    let target_bytes = serde_json::to_vec_pretty(&journal.target)?;
    if hash_bytes(&target_bytes) != journal.target_hash {
        return Err(OmmError::new(
            ErrorCode::Corrupt,
            false,
            "configuration journal target is corrupt",
        ));
    }
    if lock.revision.as_deref() != Some(&journal.target_hash) {
        lock.compare(&journal.expected)?;
        lock.publish(paths, &target_bytes)?;
    }
    let result = configuration::snapshot(paths, &super::profile::compatible(owner, "service"))?;
    journal.result = Some(result.clone());
    write_private_replace(&path, &serde_json::to_vec(&journal)?)?;
    Ok(result)
}

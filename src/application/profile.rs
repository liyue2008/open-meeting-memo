//! A persistent profile generation is explicit; opening a legacy database never initializes it.
use crate::core::{
    contracts::*,
    paths::*,
    runtime::{RuntimeGuard, RuntimeLocation},
    store::hash_bytes,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
};
use uuid::Uuid;
pub const CAPABILITIES: [&str; 7] = [
    "metadata_v1",
    "trash_v1",
    "import_mix_v1",
    "processing_v1",
    "cancellation_v1",
    "config_cas_v1",
    "media_v1",
];
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileOwner {
    pub schema_version: u32,
    pub min_reader_version: u32,
    pub generation: Uuid,
    pub profile_id: String,
}
pub fn profile_id(paths: &Paths) -> Result<String> {
    for root in [&paths.config, &paths.data, &paths.state, &paths.cache] {
        if root.exists() && root.canonicalize()? != *root {
            return Err(OmmError::invalid("profile root must be canonical"));
        }
    }
    Ok(hash_bytes(&serde_json::to_vec(&json!([
        paths.config,
        paths.data,
        paths.state,
        paths.cache
    ]))?))
}
pub fn read(paths: &Paths) -> Result<Option<ProfileOwner>> {
    let path = paths.state.join("desktop-owner.json");
    if !path.try_exists()? {
        return Ok(None);
    }
    validate_private_file(&path)?;
    let owner: ProfileOwner = serde_json::from_slice(&std::fs::read(path)?)?;
    if owner.profile_id != profile_id(paths)?
        || owner.schema_version != 1
        || owner.min_reader_version != 1
    {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "profile schema is incompatible",
        ));
    }
    Ok(Some(owner))
}
/// Called only after an explicit service start owns BOTH existing runtime/state service locks.
pub fn takeover(paths: &Paths, guard: &RuntimeGuard) -> Result<ProfileOwner> {
    let _lock = writer_lock(paths)?;
    // Explicit startup may claim a legacy profile, never downgrade a newer ownership contract.
    read(paths)?;
    let owner = ProfileOwner {
        schema_version: 1,
        min_reader_version: 1,
        generation: guard.owner.instance_id,
        profile_id: profile_id(paths)?,
    };
    write_private_replace(
        &paths.state.join("desktop-owner.json"),
        &serde_json::to_vec(&owner)?,
    )?;
    Ok(owner)
}
pub fn writer_lock(paths: &Paths) -> Result<File> {
    private_dir(&paths.config)?;
    let path = paths.config.join("profile-writers.lock");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    validate_private_file(&path)?;
    fs2::FileExt::try_lock_exclusive(&file)
        .map_err(|_| OmmError::new(ErrorCode::Conflict, true, "profile writer is busy"))?;
    Ok(file)
}
/// The stable state lock prevents an old daemon starting between offline verification and commit.
pub fn offline_lock(paths: &Paths) -> Result<File> {
    private_dir(&paths.state)?;
    let path = paths.state.join("service.lock");
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    validate_private_file(&path)?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
        OmmError::new(
            ErrorCode::Conflict,
            false,
            "a service owns this profile; use its compatible socket",
        )
    })?;
    Ok(file)
}
pub fn incompatible(state: &str, reason: &str, switch: bool) -> Value {
    json!({"protocolVersion":null,"serviceGeneration":null,"stateSchemaVersion":null,"minReaderVersion":null,"capabilities":[],"state":state,"writesAllowed":false,"reason":reason,"safeSwitchRequired":switch,"admissionMode":"none","writeAdmission":null})
}
pub fn compatible(owner: &ProfileOwner, mode: &str) -> Value {
    json!({"protocolVersion":1,"serviceGeneration":owner.generation,"stateSchemaVersion":owner.schema_version,"minReaderVersion":owner.min_reader_version,"capabilities":CAPABILITIES,"state":"compatible","writesAllowed":true,"reason":null,"safeSwitchRequired":false,"admissionMode":mode,"writeAdmission":null})
}
pub fn verify_admission(owner: &ProfileOwner, value: &Value) -> Result<()> {
    if value["ownerGeneration"] != owner.generation.to_string()
        || value["profileId"] != owner.profile_id
        || value["stateSchemaVersion"] != owner.schema_version
        || value["expiresAtMs"].as_i64().unwrap_or(0) < chrono::Utc::now().timestamp_millis()
    {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "write admission expired or belongs to another profile generation",
        ));
    }
    Ok(())
}
pub fn explicit_initialize(paths: &Paths, location: &RuntimeLocation) -> Result<ProfileOwner> {
    let guard = RuntimeGuard::acquire(location, &paths.state)?;
    if paths.state.join("state.sqlite").exists() || paths.config.join("config.json").exists() {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "existing profile requires an explicit safe service switch, not initialization",
        ));
    }
    takeover(paths, &guard)
}

/// Construct the complete frozen handshake instead of silently dropping a malformed request.
pub fn capability_request() -> super::desktop_contracts::ServiceCapabilityRequest {
    use super::desktop_contracts::{
        Capability::*, ServiceCapabilityRequest, ServiceCapabilityRequestIntent,
    };
    ServiceCapabilityRequest {
        protocol_version: 1,
        desktop_contract_version: 1,
        required_capabilities: vec![
            MetadataV1,
            TrashV1,
            ImportMixV1,
            ProcessingV1,
            CancellationV1,
            ConfigCasV1,
            MediaV1,
        ],
        intent: ServiceCapabilityRequestIntent::Mutation,
    }
}

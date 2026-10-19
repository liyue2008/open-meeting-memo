//! Unknown service state and absent saved preferences are not negative observations.
use crate::core::{contracts::*, paths::*};
use serde_json::{json, Value};
pub fn recording_preferences(paths: &Paths) -> (Option<bool>, Option<bool>) {
    let read = || -> Result<Value> {
        let path = paths.config.join("config.json");
        validate_private_file(&path)?;
        if std::fs::metadata(&path)?.len() > 128 * 1024 {
            return Err(OmmError::invalid("configuration exceeds budget"));
        }
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    };
    match read() {
        Ok(value) => (
            value["auto_record"].as_bool(),
            value["recording_consent"].as_bool(),
        ),
        Err(_) => (None, None),
    }
}
/// Read the verified service's durable recording state without invoking legacy `status`, which opens a writable Store.
pub fn service_status(
    paths: &Paths,
    runtime: &crate::core::runtime::RuntimeLocation,
    owner: &crate::core::runtime::RuntimeOwner,
) -> Result<Value> {
    let verify = || -> Result<()> {
        let current = crate::core::runtime::verify_active(&runtime.path, &paths.state)?;
        if current != *owner {
            return Err(OmmError::unavailable("service identity changed"));
        }
        Ok(())
    };
    verify()?;
    for suffix in ["-wal", "-shm", "-journal"] {
        if paths
            .state
            .join(format!("state.sqlite{suffix}"))
            .try_exists()?
        {
            return Err(OmmError::unavailable(
                "SQLite sidecars require controlled recovery",
            ));
        }
    }
    let store = crate::core::store::Store::open_read_only(paths)?;
    let recording = store.state::<Value>("recording")?.map(
        |recording| json!({"sessionId":recording["session_id"],"sourceId":recording["source_id"]}),
    );
    verify()?;
    Ok(json!({"running":true,"recording":recording,"serviceInstanceId":owner.instance_id}))
}
pub fn record_status(paths: &Paths, compatibility: Value, status: Option<Result<Value>>) -> Value {
    let (auto_record, consent) = recording_preferences(paths);
    let mut state = "unknown";
    let mut meeting = Value::Null;
    let mut error = Some(super::error(&OmmError::unavailable(
        "recording status could not be verified",
    )));
    if compatibility["admissionMode"] == "service" {
        if let Some(Ok(value)) = status {
            if value["running"] == true
                && !compatibility["serviceGeneration"].is_null()
                && value["serviceInstanceId"] == compatibility["serviceGeneration"]
            {
                if let Some(recording) = value.get("recording") {
                    if recording.is_null() {
                        state = "idle";
                        error = None;
                    } else if recording["sessionId"]
                        .as_str()
                        .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
                    {
                        state = "recording";
                        meeting = recording["sessionId"].clone();
                        error = None;
                    }
                }
            }
        }
    }
    json!({"state":state,"meetingId":meeting,"startedAtMs":null,"autoRecord":auto_record,"recordingConsent":consent,"compatibility":compatibility,"error":error})
}

//! CLI and desktop use the same capability-checked mutation boundary.
use super::{profile, validation};
use crate::{
    core::{contracts::*, paths::Paths},
    service::daemon,
};
use serde_json::{json, Value};
use uuid::Uuid;
pub async fn mutate(paths: &Paths, command: &str, mut payload: Value) -> Result<Value> {
    let owner = profile::read(paths)?.ok_or_else(|| {
        OmmError::new(
            ErrorCode::Conflict,
            false,
            "legacy profile is read-only; explicitly start the compatible service to switch safely",
        )
    })?;
    let location = paths.runtime_location(&std::env::vars().collect())?;
    let capability = daemon::Request {
        schema_version: 1,
        command: "desktop_capabilities_v1".into(),
        title: None,
        session_id: None,
        instance_id: None,
        desktop_request: None,
        desktop_capability: Some(profile::capability_request()),
    };
    let online = daemon::request_message_at(paths, &location, capability)
        .await
        .is_ok_and(|value| {
            value["compatibility"]["serviceGeneration"] == owner.generation.to_string()
                && value["compatibility"]["writesAllowed"] == true
        });
    let admission = json!({"ownerGeneration":owner.generation,"profileId":owner.profile_id,"stateSchemaVersion":1,"token":"cli_identity_bound_request","expiresAtMs":chrono::Utc::now().timestamp_millis()+300000,"mode":if online{"service"}else{"offline"}});
    payload["admission"] = admission;
    let request =
        json!({"schemaVersion":1,"requestId":Uuid::new_v4().to_string(),"payload":payload});
    validation::request(command, &request)?;
    if online {
        return daemon::request_message_at(
            paths,
            &location,
            daemon::Request {
                schema_version: 1,
                command: "desktop_dispatch_v1".into(),
                title: None,
                session_id: None,
                instance_id: Some(owner.generation),
                desktop_capability: None,
                desktop_request: Some(json!({"command":command,"request":request})),
            },
        )
        .await;
    }
    if command != "config_save" {
        return Err(OmmError::unavailable(
            "compatible running service required for this mutation",
        ));
    }
    let _service = profile::offline_lock(paths)?;
    let _writer = profile::writer_lock(paths)?;
    let current =
        profile::read(paths)?.ok_or_else(|| OmmError::unavailable("profile ownership changed"))?;
    profile::verify_admission(&current, &request["payload"]["admission"])?;
    super::configuration_journal::save(paths, &current, command, &request)
}

//! Shared, bounded desktop use cases. Native dialogs and window identity stay in the host.
pub mod artifacts;
pub mod bundle;
pub mod cli;
pub mod configuration;
pub mod configuration_journal;
pub mod desktop_contracts;
pub mod imports;
pub mod lifecycle;
pub mod local_coordination;
pub mod media;
pub mod observation;
pub mod permissions;
pub mod profile;
pub mod recovery;
pub mod validation;
use crate::core::{
    config::AppConfig,
    contracts::*,
    paths::Paths,
    store::{hash_bytes, Store},
};
use serde_json::{json, Value};
use uuid::Uuid;
pub fn error(error: &OmmError) -> Value {
    let reason = if error.message.contains("not found") {
        Some("not_found")
    } else if error.code == ErrorCode::Conflict {
        Some("stale_revision")
    } else {
        None
    };
    json!({"code":error.code,"reason":reason,"retryable":error.retryable,"message":match error.code{ErrorCode::Permission=>"Access denied; verify local permissions",ErrorCode::Conflict=>"State changed or work is active; reload and retry",ErrorCode::Corrupt=>"Artifact verification failed; retained files were not modified",ErrorCode::Unavailable=>"Required meeting, service, or dependency is unavailable",ErrorCode::ConsentRequired=>"Recording consent must be confirmed",ErrorCode::Timeout=>"Operation timed out; reconcile its persisted status",_=>"Operation could not complete; check configuration and diagnostics"},"fieldErrors":[]} )
}
pub fn id(value: &Value, key: &str) -> Result<Uuid> {
    Uuid::parse_str(value[key].as_str().unwrap_or(""))
        .map_err(|_| OmmError::invalid("invalid identifier"))
}
pub fn page(rows: Vec<Value>, payload: &Value) -> Result<Value> {
    let revision = hash_bytes(&serde_json::to_vec(&rows)?);
    let offset = crate::core::store::desktop::decode_cursor(payload["cursor"].as_str(), &revision)?;
    if offset > rows.len() {
        return Err(OmmError::new(ErrorCode::Conflict, false, "page changed"));
    }
    let end = (offset + payload["pageSize"].as_u64().unwrap_or(25) as usize).min(rows.len());
    Ok(
        json!({"items":rows[offset..end],"nextCursor":if end<rows.len(){Some(format!("{revision}:{end}"))}else{None},"revision":revision}),
    )
}
pub fn read(store: &Store, command: &str, payload: &Value) -> Result<Value> {
    match command {
        "meeting_list" => store.desktop_list(&serde_json::from_value(payload.clone())?),
        "meeting_get" => {
            let meeting = id(payload, "meetingId")?;
            let progress = store.segment_progress(meeting)?;
            Ok(
                json!({"meeting":store.desktop_row(meeting)?,"completedSegments":progress.iter().filter(|p|p.status=="done").count(),"totalSegments":progress.len(),"hasPartialTranscript":progress.iter().any(|p|p.status=="done")&&store.desktop_row(meeting)?["currentTranscriptTaskId"].is_null()}),
            )
        }
        "task_list" => store.desktop_tasks(payload),
        "task_get" => {
            let task = id(payload, "taskId")?;
            let row = store.task(task)?;
            Ok(
                json!({"task":store.desktop_task(task)?,"versions":store.desktop_versions(row.session_id)?.into_iter().filter(|v|v["artifact"]["taskId"]==task.to_string()).collect::<Vec<_>>(),"diagnostics":[]}),
            )
        }
        "artifact_versions" => page(store.desktop_versions(id(payload, "meetingId")?)?, payload),
        "artifact_read" => artifacts::read(store, payload),
        "notification_history" => {
            let records = store.notification_history(
                payload["meetingId"]
                    .as_str()
                    .map(|_| id(payload, "meetingId"))
                    .transpose()?,
                100,
            )?;
            let mut rows = Vec::new();
            for record in records {
                let value = serde_json::to_value(record)?;
                rows.push(json!({"id":value["id"],"meetingId":value["session_id"],"event":value["event"],"occurredAtMs":value["occurred_at_ms"],"status":value["status"],"visibility":"unknown","safeReason":value["code"]}));
            }
            let mut page = page(rows, payload)?;
            page.as_object_mut().unwrap().remove("revision");
            Ok(page)
        }
        _ => Err(OmmError::invalid("unsupported shared read command")),
    }
}
pub fn mutate(store: &mut Store, paths: &Paths, command: &str, payload: &Value) -> Result<Value> {
    match command {
        "meeting_update" => store.desktop_update(
            id(payload, "meetingId")?,
            payload["expectedRevision"]
                .as_i64()
                .ok_or_else(|| OmmError::invalid("missing revision"))?,
            &serde_json::from_value(payload["metadata"].clone())?,
        ),
        "meeting_trash" | "meeting_restore" => store.desktop_trash(
            &serde_json::from_value::<Vec<desktop_contracts::MeetingRevision>>(
                payload["meetings"].clone(),
            )?,
            command == "meeting_trash",
        ),
        "processing_cancel" => store.desktop_cancel(id(payload, "taskId")?),
        "processing_retry" => store.desktop_retry(id(payload, "taskId")?),
        "processing_requeue" => {
            let config = AppConfig::load(paths)?;
            store.desktop_requeue(id(payload, "taskId")?, &config.snapshot())
        }
        "processing_enqueue" => {
            let config = AppConfig::load(paths)?;
            let mut outcomes = Vec::new();
            let meetings = payload["meetingIds"]
                .as_array()
                .ok_or_else(|| OmmError::invalid("missing meeting IDs"))?;
            for value in meetings {
                let meeting = Uuid::parse_str(value.as_str().unwrap_or(""))
                    .map_err(|_| OmmError::invalid("invalid meeting identifier"))?;
                let outcome = (|| -> Result<Task> {
                    let row = store.desktop_row(meeting)?;
                    if row["trashed"] == true || store.desktop_busy(meeting)? {
                        return Err(OmmError::new(
                            ErrorCode::Conflict,
                            false,
                            "meeting has active work or is in trash",
                        ));
                    }
                    if payload["stage"] == "summary" {
                        let source = payload["sourceTranscriptTaskId"]
                            .as_str()
                            .or(row["currentTranscriptTaskId"].as_str())
                            .ok_or_else(|| {
                                OmmError::unavailable("committed transcript required")
                            })?;
                        let source = Uuid::parse_str(source)
                            .map_err(|_| OmmError::invalid("invalid source transcript"))?;
                        if store.task(source)?.session_id != meeting {
                            return Err(OmmError::invalid("transcript belongs to another meeting"));
                        }
                        store.force_summary(source, &config.version, &config.pi)
                    } else {
                        store.enqueue(
                            &store
                                .meeting(meeting)?
                                .ok_or_else(|| OmmError::unavailable("meeting not found"))?,
                            &config.snapshot(),
                            true,
                        )
                    }
                })();
                outcomes.push(match outcome{Ok(task)=>json!({"meetingId":meeting,"taskId":task.id,"operationId":task.id,"error":null}),Err(e)=>json!({"meetingId":meeting,"taskId":null,"operationId":null,"error":error(&e)})});
            }
            Ok(json!({"items":outcomes}))
        }
        _ => Err(OmmError::invalid("unsupported shared mutation command")),
    }
}
pub fn service_mutation(
    paths: &Paths,
    owner: &profile::ProfileOwner,
    generation: Option<Uuid>,
    message: Option<Value>,
) -> Result<Value> {
    if generation != Some(owner.generation) {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "service generation changed",
        ));
    }
    let message = message.ok_or_else(|| OmmError::invalid("missing desktop operation"))?;
    let command = message["command"]
        .as_str()
        .ok_or_else(|| OmmError::invalid("missing desktop command"))?;
    let request = &message["request"];
    validation::request(command, request)?;
    // Only the identity-bound service owns database writes; no host-provided SQL/path is accepted.
    let _writer = profile::writer_lock(paths)?;
    let current = profile::read(paths)?
        .ok_or_else(|| OmmError::new(ErrorCode::Conflict, false, "profile owner missing"))?;
    if current.generation != owner.generation {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "profile generation changed",
        ));
    }
    profile::verify_admission(&current, &request["payload"]["admission"])?;
    if matches!(command, "config_save" | "record_auto_set") {
        return configuration_journal::save(paths, &current, command, request);
    }
    if command == "import_commit" {
        return imports::commit(paths, &current, request);
    }
    let mut store = Store::open(paths)?;
    store.desktop_atomic(command, request, |store| {
        mutate(store, paths, command, &request["payload"])
    })
}

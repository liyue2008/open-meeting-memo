use crate::core::{
    contracts::*,
    store::{hash_bytes, Store},
};
use serde_json::{json, Value};
use uuid::Uuid;
pub fn reference(value: &Value) -> Result<(Uuid, Uuid, Stage)> {
    let meeting = Uuid::parse_str(value["meetingId"].as_str().unwrap_or(""))
        .map_err(|_| OmmError::invalid("invalid meeting identifier"))?;
    let task = Uuid::parse_str(value["taskId"].as_str().unwrap_or(""))
        .map_err(|_| OmmError::invalid("invalid task identifier"))?;
    let stage: Stage = serde_json::from_value(value["stage"].clone())?;
    Ok((meeting, task, stage))
}
pub fn full(store: &Store, reference: &Value) -> Result<(Vec<u8>, Stage)> {
    let (meeting, task, stage) = self::reference(reference)?;
    if store.task(task)?.session_id != meeting {
        return Err(OmmError::invalid(
            "artifact does not belong to this meeting",
        ));
    }
    Ok((store.read_artifact(task, stage)?, stage))
}
pub fn text(store: &Store, reference: &Value, format: &str) -> Result<Vec<u8>> {
    let (bytes, stage) = full(store, reference)?;
    if stage == Stage::Summary {
        return if format == "json" {
            Ok(serde_json::to_vec_pretty(
                &json!({"artifact":reference,"markdown":String::from_utf8(bytes).map_err(|_|OmmError::invalid("invalid summary encoding"))?}),
            )?)
        } else {
            Ok(bytes)
        };
    }
    let transcript: Transcript = serde_json::from_slice(&bytes)?;
    if format == "json" {
        return Ok(bytes);
    }
    Ok(transcript
        .segments
        .iter()
        .map(|line| {
            format!(
                "[{:02}:{:02}] {}: {}\n",
                line.start_ms / 60000,
                line.start_ms / 1000 % 60,
                serde_json::to_value(line.source)
                    .unwrap_or(Value::Null)
                    .as_str()
                    .unwrap_or("unknown"),
                line.text
            )
        })
        .collect::<String>()
        .into_bytes())
}
pub fn read(store: &Store, payload: &Value) -> Result<Value> {
    let (meeting, task, stage) = reference(&payload["artifact"])?;
    if store.task(task)?.session_id != meeting {
        return Err(OmmError::invalid(
            "artifact does not belong to this meeting",
        ));
    }
    let partial = if stage == Stage::Transcribe && !store.artifact_committed(task, stage)? {
        store.desktop_partial_artifact(task)?
    } else {
        None
    };
    let (bytes, revision, status) = if let Some((bytes, revision)) = partial {
        (bytes, revision, "partial")
    } else {
        let (bytes, _) = full(store, &payload["artifact"])?;
        let revision = hash_bytes(&bytes);
        (bytes, revision, "committed")
    };
    let cursor = payload["cursor"].as_str();
    let offset = crate::core::store::desktop::decode_cursor(cursor, &revision)?;
    let limit = payload["maxBytes"].as_u64().unwrap_or(65536) as usize;
    let mut lines = Vec::new();
    let mut text = String::new();
    let next;
    if stage == Stage::Transcribe {
        let transcript: Transcript = serde_json::from_slice(&bytes)?;
        if offset > transcript.segments.len() {
            return Err(OmmError::invalid("invalid transcript cursor"));
        }
        let mut consumed = 0;
        let mut end = offset;
        for line in &transcript.segments[offset..] {
            let row = json!({"startMs":line.start_ms,"endMs":line.end_ms,"speaker":serde_json::to_value(line.source)?.as_str().unwrap_or("unknown"),"text":line.text});
            let size = serde_json::to_vec(&row)?.len();
            if consumed + size > limit {
                if lines.is_empty() {
                    return Err(OmmError::unavailable(
                        "transcript line exceeds requested chunk budget",
                    ));
                }
                break;
            }
            lines.push(row);
            consumed += size;
            end += 1;
        }
        next = (end < transcript.segments.len()).then(|| format!("{revision}:{end}"));
    } else {
        let content = std::str::from_utf8(&bytes)
            .map_err(|_| OmmError::invalid("invalid summary encoding"))?;
        if offset > content.len() || !content.is_char_boundary(offset) {
            return Err(OmmError::invalid("invalid summary cursor"));
        }
        let mut end = (offset + limit).min(content.len());
        while !content.is_char_boundary(end) {
            end -= 1;
        }
        text = content[offset..end].into();
        next = (end < content.len()).then(|| format!("{revision}:{end}"));
    }
    Ok(
        json!({"artifact":payload["artifact"],"revision":revision,"status":status,"format":if stage==Stage::Transcribe{"transcript"}else{"markdown"},"text":text,"lines":lines,"nextCursor":next,"complete":next.is_none()}),
    )
}

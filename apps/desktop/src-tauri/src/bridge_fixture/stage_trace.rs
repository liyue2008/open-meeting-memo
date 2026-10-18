//! Opt-in fixed-stage diagnostics on the existing owned-view snapshot polling channel.
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    OnceLock,
};
use std::time::Instant;

const JS_STAGES: [&str; 27] = [
    "driver",
    "ownership",
    "library",
    "pagination",
    "filters",
    "content-search",
    "batch-export",
    "detail",
    "media-grant",
    "metadata",
    "intra-seek",
    "segment-switch",
    "cross-seek",
    "speed",
    "detail-snapshot",
    "artifacts",
    "retained-actions",
    "reprocessing",
    "import",
    "fake-capture",
    "settings",
    "queue",
    "appearance",
    "close",
    "close-arm",
    "snapshot-await",
    "native-read",
];
const STATES: [&str; 3] = ["begin", "complete", "failed"];

fn js_record(value: &Value) -> Option<Value> {
    let parts = value.as_array()?;
    if parts.len() != 4 {
        return None;
    }
    let stage = *JS_STAGES.get(usize::try_from(parts[0].as_u64()?).ok()?)?;
    let state = *STATES.get(usize::try_from(parts[1].as_u64()?).ok()?)?;
    let sequence = parts[2].as_u64()?;
    let elapsed = parts[3].as_u64()?;
    if sequence == 0 || sequence > 1_000_000 || elapsed > 900_000 {
        return None;
    }
    Some(
        json!({"kind":"fixture-stage","side":"js","stage":stage,"state":state,"sequence":sequence,"elapsedMs":elapsed}),
    )
}

pub fn receive(envelope: &Value) -> bool {
    if envelope["stageTraceEnabled"] != true {
        return false;
    }
    if let Some(records) = envelope["stageTrace"].as_array().filter(|v| v.len() <= 128) {
        for record in records.iter().filter_map(js_record) {
            eprintln!("{record}");
        }
    }
    true
}

pub enum NativeStage {
    Receive,
    InnerSize,
    MinSize,
    SetSize,
    Worker,
    Resize,
    Snapshot,
    Callback,
}
impl NativeStage {
    fn name(&self) -> &'static str {
        match self {
            Self::Receive => "snapshot-receive",
            Self::InnerSize => "snapshot-inner-size",
            Self::MinSize => "snapshot-min-size",
            Self::SetSize => "snapshot-set-size",
            Self::Worker => "snapshot-worker",
            Self::Resize => "snapshot-resize",
            Self::Snapshot => "snapshot-api",
            Self::Callback => "snapshot-callback",
        }
    }
}
pub fn native(enabled: bool, stage: NativeStage, completed: bool) {
    if !enabled {
        return;
    }
    static START: OnceLock<Instant> = OnceLock::new();
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let elapsed = START.get_or_init(Instant::now).elapsed().as_millis();
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    eprintln!(
        "{}",
        json!({"kind":"fixture-stage","side":"native","stage":stage.name(),"state":if completed {"complete"} else {"begin"},"sequence":sequence,"elapsedMs":elapsed})
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage_trace_only_accepts_bounded_fixed_numeric_records() {
        assert_eq!(
            js_record(&json!([14, 0, 1, 35])).unwrap()["stage"],
            "detail-snapshot"
        );
        for value in [
            json!(["secret", 0, 1, 1]),
            json!([27, 0, 1, 1]),
            json!([0, 3, 1, 1]),
            json!([0, 0, 0, 1]),
            json!([0, 0, 1, 900001]),
            json!([0, 0, 1, -1]),
            json!([0, 0, 1, 1, "payload"]),
        ] {
            assert!(js_record(&value).is_none());
        }
        assert!(!receive(&json!({"stageTrace":[[0,0,1,1]]})));
    }
}

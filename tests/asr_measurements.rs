use omm::core::contracts::*;
use serde_json::json;

fn event() -> serde_json::Value {
    json!({"type":"chunk_progress","schema_version":1,"task_id":uuid::Uuid::new_v4(),"attempt":1,"source":"mic","completed_chunks":1,"vad_spans":1,"speech_samples":3200,"processing_ms":40,
    "chunk_timing":{"phase":"native_decode_total","input_index":0,"window_index":0,"core_start_sample":1600,"decode_start_sample":0,"core_samples":3200,"decode_samples":8000,"cache_hit":false,"cache_lookup_us":3,"native_decode_wall_us":20000,"process_cpu_us":35000,"checkpoint_write_us":4,"token_count":2,"output_chars":7}})
}
#[test]
fn chunk_timing_survives_progress_json_without_content() {
    let value = event();
    let parsed: AsrWorkerEvent = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(parsed).unwrap()["chunk_timing"],
        value["chunk_timing"]
    );
}
#[test]
fn malformed_optional_timing_is_discarded_not_a_business_error() {
    for bad in [
        json!({"text":"PRIVATE_DIAGNOSTIC_MARKER"}),
        json!({"native_decode_wall_us":-1}),
        json!("PRIVATE_DIAGNOSTIC_MARKER"),
    ] {
        let mut value = event();
        value["chunk_timing"] = bad.clone();
        value["startup_timing"] = bad;
        let parsed: AsrWorkerEvent = serde_json::from_value(value).unwrap();
        let safe = serde_json::to_string(&parsed).unwrap();
        assert!(!safe.contains("PRIVATE_DIAGNOSTIC_MARKER"));
    }
}
use omm::core::{
    paths::{private_dir, Paths},
    store::{hash_file, Store},
};
fn queued_fixture() -> (tempfile::TempDir, Paths, Store, Task) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("home"), &Default::default()).unwrap();
    paths.create_private_dirs().unwrap();
    let id = uuid::Uuid::new_v4();
    let dir = paths.data.join("meetings").join(id.to_string());
    private_dir(&dir).unwrap();
    let audio = dir.join("generated.wav");
    let mut w = hound::WavWriter::create(
        &audio,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..480 {
        w.write_sample(0i16).unwrap();
    }
    w.finalize().unwrap();
    let meeting = SealedMeeting {
        session_id: id,
        title: "generated".into(),
        directory: dir,
        segments: vec![AudioSegment {
            capture: None,
            path: audio.clone(),
            duration_ms: 10,
            sha256: hash_file(&audio).unwrap(),
        }],
        sealed_at_ms: 1,
    };
    let mut store = Store::open(&paths).unwrap();
    let task = store
        .enqueue(
            &meeting,
            &ConfigSnapshot {
                asr: AsrConfig {
                    model_version: "measurement-fixture".into(),
                    timeout_ms: 10000,
                    ..Default::default()
                },
                pi: PiConfig {
                    provider: "fake".into(),
                    model: "fake".into(),
                    prompt_version: "1".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
            false,
        )
        .unwrap();
    (temp, paths, store, task)
}
fn fixture() -> (tempfile::TempDir, Paths, Store, ClaimedTask) {
    let (temp, paths, mut store, _) = queued_fixture();
    let claim = store
        .claim(chrono::Utc::now().timestamp_millis(), 60000)
        .unwrap()
        .unwrap();
    (temp, paths, store, claim)
}
fn measured(c: &TaskContext, index: u32) -> AsrWorkerEvent {
    let mut v = event();
    v["task_id"] = json!(c.task_id);
    v["attempt"] = json!(c.attempt);
    v["chunk_timing"]["window_index"] = json!(index);
    serde_json::from_value(v).unwrap()
}
#[test]
fn persisted_ring_caps_bytes_aggregates_and_rejects_stale_lease_without_readonly_migration() {
    let (_temp, paths, mut store, claim) = fixture();
    let old = Store::open_read_only(&paths).unwrap();
    assert!(old.asr_measurements(claim.task.id).unwrap()["samples"]
        .as_array()
        .unwrap()
        .is_empty());
    drop(old);
    assert_eq!(
        task_show(&_temp, claim.task.id)["totals"]["observations"],
        0
    );
    let db = rusqlite::Connection::open(paths.state.join("state.sqlite")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='asr_measurements'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    for i in 0..300 {
        store.record_asr_measurement_best_effort(&claim.context, &measured(&claim.context, i));
    }
    let result = store.asr_measurements(claim.task.id).unwrap();
    assert_eq!(result["samples"].as_array().unwrap().len(), 256);
    assert_eq!(result["totals"]["observations"], 300);
    assert_eq!(result["totals"]["native_decode_wall_us"], 6000000);
    for sample in result["samples"].as_array().unwrap() {
        assert!(serde_json::to_vec(sample).unwrap().len() <= 1024);
        assert_eq!(sample["lease"], claim.lease.token.to_string());
    }
    store.record_asr_measurement_best_effort(&claim.context, &measured(&claim.context, 299));
    assert_eq!(
        store.asr_measurements(claim.task.id).unwrap()["totals"]["observations"],
        300
    );
    let now = chrono::Utc::now().timestamp_millis();
    let failure = OmmError::new(ErrorCode::Timeout, true, "generated");
    store.fail(&claim, &failure, now).unwrap();
    store
        .schedule_retry(&store.task(claim.task.id).unwrap(), now)
        .unwrap();
    let retry = store.claim(now + 2000, 60000).unwrap().unwrap();
    assert_ne!(retry.lease.token, claim.lease.token);
    store.record_asr_measurement_best_effort(&claim.context, &measured(&claim.context, 301));
    assert_eq!(
        store.asr_measurements(claim.task.id).unwrap()["totals"]["observations"],
        300
    );
    store.record_asr_measurement_best_effort(&retry.context, &measured(&retry.context, 0));
    assert_eq!(
        store.asr_measurements(claim.task.id).unwrap()["totals"]["observations"],
        301
    );
    assert_eq!(
        task_show(&_temp, claim.task.id)["totals"]["observations"],
        301
    );
    let reader = Store::open_read_only(&paths).unwrap();
    assert_eq!(
        reader.asr_measurements(claim.task.id).unwrap()["totals"]["observations"],
        301
    );
}
fn task_show(temp: &tempfile::TempDir, task: uuid::Uuid) -> serde_json::Value {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_omm"))
        .env_clear()
        .env("HOME", temp.path().join("home"))
        .env("XDG_RUNTIME_DIR", temp.path().canonicalize().unwrap())
        .args(["task", "show", &task.to_string(), "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    value["data"]["asrMeasurements"].clone()
}
#[test]
fn optional_measurement_limits_reject_bad_ratios_indices_and_cache_hit_cost() {
    for (key, bad) in [
        ("input_index", json!(4096)),
        ("window_index", json!(1000000)),
        ("native_decode_wall_us", json!(86400000001u64)),
        ("process_cpu_us", json!(999999999999u64)),
        ("decode_samples", json!(400001)),
        ("cache_hit", json!(true)),
        ("output_chars", json!("PRIVATE_MARKER")),
    ] {
        let mut v = event();
        v["chunk_timing"][key] = bad;
        let parsed: AsrWorkerEvent = serde_json::from_value(v).unwrap();
        assert!(
            serde_json::to_value(parsed)
                .unwrap()
                .get("chunk_timing")
                .is_none(),
            "{key}"
        );
    }
}
#[tokio::test]
async fn metric_database_reject_keeps_real_worker_commit_and_next_task_healthy() {
    use omm::adapters::asr::LocalAsrAdapter;
    use std::os::unix::fs::PermissionsExt;
    let (_temp, paths, mut store, claim) = fixture();
    store.record_asr_measurement_best_effort(&claim.context, &measured(&claim.context, 0));
    let db = rusqlite::Connection::open(paths.state.join("state.sqlite")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_measurements BEFORE INSERT ON asr_measurements BEGIN SELECT RAISE(FAIL,'PRIVATE_DB_MARKER'); END;").unwrap();
    let terminal = AsrWorkerEvent::Result {
        schema_version: 1,
        task_id: claim.task.id,
        attempt: claim.context.attempt,
        transcript: Transcript {
            schema_version: 1,
            model_version: claim.context.config.asr.model_version.clone(),
            duration_ms: 10,
            segments: vec![],
        },
    };
    let worker = claim.context.meeting_dir.join("controlled-worker");
    std::fs::write(
        &worker,
        format!(
            "#!/bin/sh\n/bin/cat >/dev/null\nprintf '%s\\n' '{}' '{}'\n",
            serde_json::to_string(&measured(&claim.context, 1)).unwrap(),
            serde_json::to_string(&terminal).unwrap()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let asr = LocalAsrAdapter::with_paths(worker, paths.clone()).unwrap();
    let result = asr
        .transcribe(AsrRequest {
            context: claim.context.clone(),
            segments: claim.segments.clone(),
        })
        .await
        .unwrap();
    asr.drain().await.unwrap();
    store
        .commit_transcript(&claim, &result, chrono::Utc::now().timestamp_millis())
        .unwrap();
    assert_eq!(
        store.task(claim.task.id).unwrap().status,
        TaskStatus::TranscriptCommitted
    );
    let summary = store
        .claim(chrono::Utc::now().timestamp_millis(), 60000)
        .unwrap()
        .unwrap();
    store
        .commit_summary(
            &summary,
            &Summary {
                schema_version: 1,
                markdown: "generated".into(),
                provider: "fake".into(),
                model: "fake".into(),
                prompt_version: "1".into(),
            },
            chrono::Utc::now().timestamp_millis(),
        )
        .unwrap();
    assert_eq!(
        store.task(claim.task.id).unwrap().status,
        TaskStatus::Completed
    );
    let meeting = store.meeting(claim.task.session_id).unwrap().unwrap();
    let next = store
        .enqueue(&meeting, &ConfigSnapshot::default(), true)
        .unwrap();
    assert_eq!(
        store
            .claim(chrono::Utc::now().timestamp_millis(), 60000)
            .unwrap()
            .unwrap()
            .task
            .id,
        next.id
    );
}
#[tokio::test]
async fn unrepresentable_optional_number_cannot_fail_valid_business_result() {
    use omm::adapters::asr::LocalAsrAdapter;
    use std::os::unix::fs::PermissionsExt;
    let (_temp, _paths, _store, claim) = fixture();
    let raw = serde_json::to_string(&measured(&claim.context, 0))
        .unwrap()
        .replace(
            "\"native_decode_wall_us\":20000",
            "\"native_decode_wall_us\":1e999",
        );
    assert!(raw.contains("1e999"));
    let terminal = AsrWorkerEvent::Result {
        schema_version: 1,
        task_id: claim.task.id,
        attempt: claim.context.attempt,
        transcript: Transcript {
            schema_version: 1,
            model_version: claim.context.config.asr.model_version.clone(),
            duration_ms: 10,
            segments: vec![],
        },
    };
    let worker = claim.context.meeting_dir.join("huge-number-worker");
    std::fs::write(
        &worker,
        format!(
            "#!/bin/sh\n/bin/cat >/dev/null\nprintf '%s\\n' '{}' '{}'\n",
            raw,
            serde_json::to_string(&terminal).unwrap()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adapter = LocalAsrAdapter::new(worker).unwrap();
    adapter
        .transcribe(AsrRequest {
            context: claim.context,
            segments: claim.segments,
        })
        .await
        .unwrap();
    adapter.drain().await.unwrap();
}
#[test]
fn broken_diagnostic_stderr_and_metric_rejection_do_not_abort_business() {
    use std::{
        os::fd::{FromRawFd, OwnedFd},
        process::{Command, Stdio},
    };
    let mut fds = [-1; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
    unsafe {
        libc::close(fds[0]);
    }
    let writer = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "metric_database_reject_keeps_real_worker_commit_and_next_task_healthy",
            "--nocapture",
        ])
        .stderr(Stdio::from(writer))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
#[tokio::test]
async fn four_thousand_oversized_optional_progress_records_do_not_fill_terminal_output_budget() {
    use omm::adapters::asr::LocalAsrAdapter;
    use std::os::unix::fs::PermissionsExt;
    let (_temp, _paths, _store, claim) = fixture();
    let mut value = serde_json::to_value(measured(&claim.context, 0)).unwrap();
    value["chunk_timing"] = json!({"text":"PRIVATE_STREAM_MARKER".repeat(1024)});
    let terminal = AsrWorkerEvent::Result {
        schema_version: 1,
        task_id: claim.task.id,
        attempt: claim.context.attempt,
        transcript: Transcript {
            schema_version: 1,
            model_version: claim.context.config.asr.model_version.clone(),
            duration_ms: 10,
            segments: vec![],
        },
    };
    let worker = claim.context.meeting_dir.join("large-progress-worker");
    std::fs::write(&worker,format!("#!/bin/sh\n/bin/cat >/dev/null\ni=0\nwhile [ $i -lt 4096 ]; do printf '%s\\n' '{}'; i=$((i+1)); done\nprintf '%s\\n' '{}'\n",serde_json::to_string(&value).unwrap(),serde_json::to_string(&terminal).unwrap())).unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adapter = LocalAsrAdapter::new(worker).unwrap();
    let mut context = claim.context;
    context.config.asr.timeout_ms = 30000;
    let result = adapter
        .transcribe(AsrRequest {
            context,
            segments: claim.segments,
        })
        .await
        .unwrap();
    assert!(result.segments.is_empty());
    adapter.drain().await.unwrap();
}

fn terminal(context: &TaskContext) -> AsrWorkerEvent {
    AsrWorkerEvent::Result {
        schema_version: 1,
        task_id: context.task_id,
        attempt: context.attempt,
        transcript: Transcript {
            schema_version: 1,
            model_version: context.config.asr.model_version.clone(),
            duration_ms: 10,
            segments: vec![],
        },
    }
}
async fn raw_worker(claim: &ClaimedTask, raw: &str) -> Result<Transcript> {
    use omm::adapters::asr::LocalAsrAdapter;
    use std::os::unix::fs::PermissionsExt;
    let events = claim.context.meeting_dir.join("protocol-events");
    std::fs::write(&events, raw).unwrap();
    let worker = claim.context.meeting_dir.join("protocol-worker");
    std::fs::write(
        &worker,
        format!(
            "#!/bin/sh\n/bin/cat >/dev/null\nexec /bin/cat '{}'\n",
            events.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adapter = LocalAsrAdapter::new(worker).unwrap();
    let result = adapter
        .transcribe(AsrRequest {
            context: claim.context.clone(),
            segments: claim.segments.clone(),
        })
        .await;
    adapter.drain().await.unwrap();
    result
}
fn insert_field(raw: &str, field: &str, first: bool) -> String {
    if first {
        format!("{{{field},{}", &raw[1..])
    } else {
        format!("{},{field}}}", &raw[..raw.len() - 1])
    }
}
#[tokio::test]
async fn optional_observers_cannot_hide_duplicate_business_keys_in_either_order() {
    let (_temp, _paths, _store, claim) = fixture();
    let result = serde_json::to_string(&terminal(&claim.context)).unwrap();
    let progress = serde_json::to_string(&measured(&claim.context, 0)).unwrap();
    for first in [true, false] {
        for observer in [
            "\"startup_timing\":null",
            "\"chunk_timing\":{\"native_decode_wall_us\":1e999}",
        ] {
            for (raw, field) in [
                (result.as_str(), "\"attempt\":999".to_owned()),
                (result.as_str(), "\"\\u0061ttempt\":999".to_owned()),
                (
                    result.as_str(),
                    format!("\"task_id\":\"{}\"", uuid::Uuid::new_v4()),
                ),
                (
                    result.as_str(),
                    "\"type\":\"PRIVATE_PROTOCOL_MARKER\"".to_owned(),
                ),
                (
                    progress.as_str(),
                    "\"source\":\"PRIVATE_PROTOCOL_MARKER\"".to_owned(),
                ),
            ] {
                let raw = insert_field(&insert_field(raw, &field, first), observer, true);
                let error = raw_worker(&claim, &format!("{raw}\n{result}\n"))
                    .await
                    .expect_err("duplicate business field accepted through observer fallback");
                assert_eq!(error.code, ErrorCode::Corrupt);
                assert!(!error.message.contains("PRIVATE_PROTOCOL_MARKER"));
            }
            let segment = "{\"source\":\"mic\",\"startMs\":0,\"endMs\":10,\"text\":\"PRIVATE_PROTOCOL_MARKER\"}";
            let duplicate = insert_field(segment, "\"source\":\"system\"", first);
            let raw = result.replace("\"segments\":[]", &format!("\"segments\":[{duplicate}]"));
            let raw = insert_field(&raw, observer, true);
            let error = raw_worker(&claim, &format!("{raw}\n")).await.unwrap_err();
            assert_eq!(error.code, ErrorCode::Corrupt);
            assert!(!error.message.contains("PRIVATE_PROTOCOL_MARKER"));
        }
    }
}
#[tokio::test]
async fn observer_projection_keeps_business_protocol_and_resource_errors_strict() {
    let (_temp, _paths, _store, claim) = fixture();
    let result = serde_json::to_string(&terminal(&claim.context)).unwrap();
    for raw in [
        result.replace(
            "\"type\":\"result\"",
            "\"type\":\"PRIVATE_PROTOCOL_MARKER\"",
        ),
        result.replacen("\"schema_version\":1", "\"schema_version\":999", 1),
        result.replace("\"transcript\":", "\"PRIVATE_PROTOCOL_MARKER\":"),
        result.replace(
            "\"segments\":[]",
            "\"segments\":\"PRIVATE_PROTOCOL_MARKER\"",
        ),
        insert_field(&result, "\"business_unknown\":1e999", true),
        format!("{result}PRIVATE_PROTOCOL_MARKER"),
        "{PRIVATE_PROTOCOL_MARKER".into(),
    ] {
        let raw = insert_field(&raw, "\"startup_timing\":null", true);
        let error = raw_worker(&claim, &format!("{raw}\n")).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Corrupt);
        assert!(!error.message.contains("PRIVATE_PROTOCOL_MARKER"));
    }
    // Optional diagnostics can be malformed, duplicated or unrepresentable; business stays valid.
    for observer in [
        "\"startup_timing\":1e999",
        "\"chunk_timing\":\"PRIVATE_PROTOCOL_MARKER\"",
        "\"startup_timing\":null,\"startup_timing\":1e999,\"chunk_timing\":null,\"chunk_timing\":null",
    ] {
        let raw = insert_field(&result, observer, true);
        let transcript = raw_worker(&claim, &format!("{raw}\n")).await.unwrap();
        assert!(transcript.segments.is_empty());
    }
    raw_worker(&claim, &format!("{result}\n")).await.unwrap(); // Legacy worker.
    let raw = insert_field(&result, "\"startup_timing\":null", true);
    let error = raw_worker(&claim, &format!("{raw}\n{raw}\n"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Corrupt); // Exactly one terminal event.
    let huge = insert_field(
        &result,
        &format!("\"chunk_timing\":\"{}\"", "x".repeat(16 * 1024 * 1024)),
        true,
    );
    let error = raw_worker(&claim, &format!("{huge}\n")).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Backend);
    assert!(error.message.contains("line resource limit"));
}

struct UnusedSummary;
#[async_trait::async_trait]
impl SummaryAdapter for UnusedSummary {
    async fn summarize(&self, _: SummaryRequest) -> Result<Summary> {
        panic!("transcription fixture must not call a summary provider")
    }
}
fn paced_worker(
    root: &std::path::Path,
    task: &Task,
    startup_delay_ms: u64,
) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let pid = root.join("owned-worker.pid");
    let worker = root.join("paced-worker");
    let mut script = format!(
        "#!/bin/sh\n/bin/cat >/dev/null\n/bin/sleep {:.3}\nprintf '%s\\n' $$ > '{}'\n",
        startup_delay_ms as f64 / 1000.0,
        pid.display()
    );
    for i in 0..70 {
        let mut v = event();
        v["task_id"] = json!(task.id);
        v["attempt"] = json!(task.attempt);
        v["completed_chunks"] = json!(i);
        v["vad_spans"] = json!(70);
        v["processing_ms"] = json!(i * 50);
        v["chunk_timing"]["window_index"] = json!(i);
        script.push_str(&format!("printf '%s\\n' '{}'\n/bin/sleep 0.05\n", v));
    }
    let result = json!({"type":"result","schema_version":1,"task_id":task.id,"attempt":task.attempt,"transcript":{"schemaVersion":1,"modelVersion":"measurement-fixture","durationMs":10,"segments":[]}});
    script.push_str(&format!("printf '%s\\n' '{result}'\n"));
    std::fs::write(&worker, script).unwrap();
    std::fs::set_permissions(&worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    (worker, pid)
}
#[tokio::test]
async fn paced_owned_progress_renews_one_second_lease_past_initial_expiry_and_cancels_promptly() {
    use omm::{adapters::asr::LocalAsrAdapter, core::pipeline};
    use std::time::{Duration, Instant};
    for cancel in [false, true] {
        let (temp, paths, mut store, task) = queued_fixture();
        let (worker, pid_path) = paced_worker(temp.path(), &task, if cancel { 950 } else { 0 });
        let adapter = LocalAsrAdapter::with_paths(worker, paths.clone()).unwrap();
        let db = rusqlite::Connection::open_with_flags(
            paths.state.join("state.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let started = Instant::now();
        let deadline = tokio::time::sleep(Duration::from_secs(8));
        tokio::pin!(deadline);
        let mut operation = Box::pin(pipeline::run_one(
            &mut store,
            &adapter,
            &UnusedSummary,
            1000,
        ));
        let mut tick = tokio::time::interval(Duration::from_millis(25));
        let mut first_expiry = None;
        let mut last_expiry = None;
        let mut last_renewal = Instant::now();
        let mut max_renewal_gap = Duration::ZERO;
        let mut renewals = 0;
        let mut completed = None;
        let mut ready = None;
        let mut child_pid = None;
        let mut metrics_table_ready = false;
        loop {
            tokio::select! {
                result = &mut operation => {completed = Some(result.unwrap().unwrap());break;},
                _ = &mut deadline => panic!("paced worker exceeded bounded fixture deadline"),
                _ = tick.tick() => {
                    let expiry: Option<i64> = db.query_row("SELECT expires FROM tasks WHERE id=?1", [task.id.to_string()], |r| r.get(0)).unwrap();
                    if let Some(expiry) = expiry {
                        assert!(expiry > chrono::Utc::now().timestamp_millis(), "live paced progress allowed lease to expire");
                        first_expiry.get_or_insert(expiry);
                        if last_expiry != Some(expiry) {
                            max_renewal_gap = max_renewal_gap.max(last_renewal.elapsed());
                            last_renewal = Instant::now();
                            last_expiry = Some(expiry);
                            renewals += 1;
                        }
                    }
                    // Startup and persistence are not part of the paced observation window.
                    if child_pid.is_none() {
                        if let Ok(line)=std::fs::read_to_string(&pid_path) {
                            if line.len()<=32 && line.ends_with('\n') {
                                if let Ok(pid)=line.trim().parse::<i32>() {
                                    assert!(pid>0);assert_eq!(unsafe {libc::kill(pid,0)},0,"fixture child must be live at readiness");child_pid=Some(pid);
                                }
                            }
                        }
                    }
                    if !metrics_table_ready {metrics_table_ready=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='asr_measurement_totals')",[],|r|r.get::<_,bool>(0)).unwrap();}
                    let observations = if metrics_table_ready {
                        use rusqlite::OptionalExtension;
                        let totals:Option<String>=db.query_row("SELECT document FROM asr_measurement_totals WHERE task=?1",[task.id.to_string()],|r|r.get(0)).optional().unwrap();
                        totals.map(|s|serde_json::from_str::<serde_json::Value>(&s).unwrap()["observations"].as_u64().unwrap()).unwrap_or(0)
                    } else {0};
                    if child_pid.is_some() && observations>0 {ready.get_or_insert_with(Instant::now);}
                    if cancel && ready.is_some_and(|t|t.elapsed()>=Duration::from_millis(1300)) && observations>=10 {
                        assert_eq!(unsafe {libc::kill(child_pid.unwrap(),0)},0,"cancel while producer is still live");break;
                    }
                }
            }
        }
        drop(operation);
        let drain_start = Instant::now();
        tokio::time::timeout(Duration::from_secs(2), adapter.drain())
            .await
            .unwrap()
            .unwrap();
        let elapsed = started.elapsed();
        if let Some(task) = &completed {
            assert_eq!(
                task.status,
                TaskStatus::TranscriptCommitted,
                "paced task error: {:?}",
                task.error
            );
        }
        assert!(renewals >= 4);
        assert!(max_renewal_gap < Duration::from_millis(900));
        assert!(last_expiry.unwrap() - first_expiry.unwrap() >= 900);
        let samples = store.asr_measurements(task.id).unwrap();
        let observations = samples["totals"]["observations"].as_u64().unwrap();
        assert!(observations >= if cancel { 10 } else { 70 });
        let pid: i32 = std::fs::read_to_string(pid_path)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(child_pid, Some(pid));
        assert!(
            ready.is_some(),
            "persisted progress must be observed while child is live"
        );
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "owned child must be waited before drain returns"
        );
        if cancel {
            assert!(completed.is_none());
            assert!(drain_start.elapsed() < Duration::from_secs(2));
        } else {
            assert_eq!(completed.unwrap().status, TaskStatus::TranscriptCommitted);
            assert!(elapsed >= Duration::from_millis(3500));
            let artifact: Transcript =
                serde_json::from_slice(&store.read_artifact(task.id, Stage::Transcribe).unwrap())
                    .unwrap();
            assert_eq!(artifact.duration_ms, 10);
        }
        assert!(paths
            .data
            .join("meetings")
            .join(task.session_id.to_string())
            .join("generated.wav")
            .exists());
        println!("paced cancel={cancel} elapsed_ms={} renewals={renewals} max_observed_renewal_gap_ms={} observations={observations} drain_ms={}", elapsed.as_millis(), max_renewal_gap.as_millis(), drain_start.elapsed().as_millis());
    }
}

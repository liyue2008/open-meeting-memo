//! Explicit review-only producers; real Core state/claims and the normal capture transport.
use super::*;
use omm::adapters::capture::{CapturePaths, MacCaptureAdapter};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    sync::Mutex,
    time::{Duration, Instant},
};

pub fn mode() -> Option<String> {
    let mode = std::env::var("OMM_NATIVE_REVIEW_FIXTURE").ok()?;
    assert_eq!(
        scenario(),
        "ui",
        "review producers are restricted to the explicit private UI scenario"
    );
    assert!(
        ["cold-recording", "partial-transcript"].contains(&mode.as_str()),
        "unknown review fixture"
    );
    Some(mode)
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn configure(paths: &Paths, cfg: &mut AppConfig) {
    if mode().as_deref() != Some("cold-recording") {
        return;
    }
    let root = output("driver.js").parent().unwrap().to_path_buf();
    let home = root.join("home");
    private_dir(&home).unwrap();
    let runtime = paths
        .runtime_location(&std::env::vars().collect())
        .unwrap()
        .path;
    let mut environment = vec![
        ("HOME", home.display().to_string()),
        ("PATH", "/usr/bin:/bin:/usr/sbin:/sbin".into()),
        ("OMM_NATIVE_FIXTURE_ROOT", root.display().to_string()),
        ("OMM_NATIVE_FIXTURE_HELPER", "1".into()),
        ("OMM_NATIVE_REVIEW_CAPTURE", "1".into()),
    ];
    for key in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        let value = std::env::var(key).unwrap();
        let canonical = PathBuf::from(&value).canonicalize().unwrap();
        assert!(canonical.starts_with(&root));
        environment.push((key, canonical.display().to_string()));
    }
    write_private_new(&output("review-capture-pids"), b"").unwrap();
    write_private_new(&output("review-capture-events.jsonl"), b"").unwrap();
    let executable = std::env::current_exe().unwrap().canonicalize().unwrap();
    let script=format!("#!/bin/sh\nset -eu\numask 077\n[ \"$YULU_IPC_DIR\" = {} ]\n[ \"$OMM_SERVICE_SOCKET\" = {} ]\nprintf '%s\\n' \"$$\" >> {}\nexec /usr/bin/env -i {} OMM_NATIVE_FIXTURE_CAPTURE_INSTANCE=\"$YULU_SERVICE_OWNER\" {}\n",quote(&runtime.join("capture").display().to_string()),quote(&runtime.join("service.sock").display().to_string()),quote(&output("review-capture-pids").display().to_string()),environment.iter().map(|(k,v)|format!("{k}={}",quote(v))).collect::<Vec<_>>().join(" "),quote(&executable.display().to_string()));
    let launcher = output("review-capture-launcher");
    write_private_new(&launcher, script.as_bytes()).unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    cfg.capture_executable = launcher;
    cfg.auto_record = false;
    cfg.notifications.enabled = false;
}
pub fn validate_capture_context(paths: &Paths, instance: Uuid) {
    if std::env::var_os("OMM_NATIVE_REVIEW_CAPTURE").is_none() {
        return;
    }
    let root = output("driver.js").parent().unwrap().to_path_buf();
    for path in [&paths.data, &paths.state, &paths.config, &paths.cache] {
        assert!(path.canonicalize().unwrap().starts_with(&root));
        validate_private_dir(path).unwrap();
    }
    assert_eq!(
        std::fs::canonicalize(std::env::var_os("HOME").unwrap()).unwrap(),
        root.join("home")
    );
    let cfg = AppConfig::load(paths).unwrap();
    assert!(!cfg.notifications.enabled && !cfg.auto_record);
    assert_eq!(cfg.capture_executable, output("review-capture-launcher"));
    assert_eq!(
        Store::open_read_only(paths)
            .unwrap()
            .state::<Uuid>("capture_instance")
            .unwrap(),
        Some(instance)
    );
    let scenario: Value =
        serde_json::from_slice(&std::fs::read(output("review-scenario.json")).unwrap()).unwrap();
    assert_eq!(scenario["mode"], "cold-recording");
}
pub fn capture_event(event: Value) {
    if std::env::var_os("OMM_NATIVE_REVIEW_CAPTURE").is_none() {
        return;
    }
    let path = output("review-capture-events.jsonl");
    validate_private_file(&path).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() < 64 * 1024);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .unwrap();
    writeln!(file, "{event}").unwrap();
    file.sync_all().unwrap();
}
fn segment(meeting: &SealedMeeting, index: u32) -> Result<AudioSegment> {
    let path = meeting.directory.join(format!("segment-{index}.wav"));
    let mut bytes = media::silent_wav(6);
    bytes[22..24].copy_from_slice(&2u16.to_le_bytes());
    bytes[24..28].copy_from_slice(&48000u32.to_le_bytes());
    bytes[28..32].copy_from_slice(&192000u32.to_le_bytes());
    bytes[32..34].copy_from_slice(&4u16.to_le_bytes());
    write_private_new(&path, &bytes)?;
    Ok(AudioSegment {
        capture: None,
        path,
        sha256: hash_bytes(&bytes),
        duration_ms: 1000,
    })
}
fn commit_segment(store: &mut Store, meeting: Uuid, index: u32) -> Result<()> {
    let document = store
        .meeting(meeting)?
        .ok_or_else(|| OmmError::invalid("missing synthetic partial meeting"))?;
    let audio = segment(&document, index)?;
    store.admit_segment(meeting, index, &audio)?;
    let now = chrono::Utc::now().timestamp_millis();
    let claim = store
        .claim_segment(now, 30_000)?
        .ok_or_else(|| OmmError::unavailable("synthetic segment claim unavailable"))?;
    if claim.context.session_id != meeting {
        return Err(OmmError::invalid("refusing another meeting's segment"));
    }
    let transcript = Transcript {
        schema_version: 1,
        model_version: claim.context.config.asr.model_version.clone(),
        duration_ms: 1000,
        segments: (0..3)
            .map(|i| TranscriptSegment {
                source: AudioSource::Mic,
                start_ms: i * 200,
                end_ms: i * 200 + 100,
                text: format!("Review partial segment {index} synthetic line {i}"),
            })
            .collect(),
    };
    store.commit_segment(&claim, &transcript, now + 1)
}
pub fn seed(paths: &Paths) {
    let Some(mode) = mode() else {
        return;
    };
    paths.runtime_location(&std::env::vars().collect()).unwrap();
    let mut result =
        json!({"mode":mode,"deadlineSeconds":150,"maxControls":32,"nonpersistent":true});
    if mode == "partial-transcript" {
        let meeting = Uuid::parse_str("00000000-0000-4000-8000-000000000042").unwrap();
        let directory = paths.data.join("meetings").join(meeting.to_string());
        private_dir(&directory).unwrap();
        let item = SealedMeeting {
            session_id: meeting,
            title: "Review real partial transcript".into(),
            directory,
            segments: vec![],
            sealed_at_ms: 0,
        };
        let mut store = Store::open(paths).unwrap();
        let task = store
            .begin_segmented(&item, &AppConfig::load(paths).unwrap().snapshot())
            .unwrap();
        commit_segment(&mut store, meeting, 0).unwrap();
        assert!(!store
            .artifact_committed(task.id, Stage::Transcribe)
            .unwrap());
        result["meetingId"] = json!(meeting);
        result["taskId"] = json!(task.id);
        result["artifact"] = json!({"meetingId":meeting,"taskId":task.id,"stage":"transcribe"});
        result["initialPartialRevision"] =
            json!(store.desktop_partial_artifact(task.id).unwrap().unwrap().1);
    } else {
        let runtime = paths
            .runtime_location(&std::env::vars().collect())
            .unwrap()
            .path;
        assert!(!runtime.join("capture/audio_daemon.sock").exists());
        assert!(std::fs::read(output("review-capture-pids"))
            .unwrap()
            .is_empty());
        result["coldBaseline"] = json!({"helperSocketAbsent":true,"captureNotSpawned":true});
    }
    write_private_new(
        &output("review-scenario.json"),
        &serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: String,
    action: String,
}
fn valid(request: &Request) -> bool {
    !request.id.is_empty()
        && request.id.len() <= 64
        && request
            .id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        && ["inspect", "advance", "finalize", "restore", "finish"]
            .contains(&request.action.as_str())
}
pub fn cleanup_without_window() {
    assert_eq!(mode().as_deref(), Some("partial-transcript"));
    let root = output("driver.js").parent().unwrap().to_path_buf();
    validate_private_file(&output("review-scenario.json")).unwrap();
    let paths = Paths::from_env().unwrap();
    for path in [&paths.data, &paths.state, &paths.config, &paths.cache] {
        assert!(path.canonicalize().unwrap().starts_with(&root));
        validate_private_dir(path).unwrap();
    }
    let cfg = AppConfig::load(&paths).unwrap();
    assert!(!cfg.notifications.enabled && !cfg.auto_record);
    let mut controller = Controller::new();
    controller.finalize(&paths).unwrap();
    write_private_new(
        &output("review-startup-cleanup.json"),
        &serde_json::to_vec(&controller.inspect(&paths).unwrap()).unwrap(),
    )
    .unwrap();
}
pub struct Controller {
    mode: Option<String>,
    created: Instant,
    ids: BTreeSet<String>,
    count: u32,
    phase: u8,
    finished: bool,
    receipts: Vec<Value>,
}
impl Controller {
    pub fn new() -> Self {
        Self {
            mode: mode(),
            created: Instant::now(),
            ids: BTreeSet::new(),
            count: 0,
            phase: 0,
            finished: false,
            receipts: vec![],
        }
    }
    pub fn expired(&self) -> bool {
        self.mode.is_some() && !self.finished && self.created.elapsed() > Duration::from_secs(150)
    }
    fn partial_ids(&self) -> Result<(Uuid, Uuid)> {
        let data: Value = serde_json::from_slice(&std::fs::read(output("review-scenario.json"))?)?;
        Ok((
            serde_json::from_value(data["meetingId"].clone())?,
            serde_json::from_value(data["taskId"].clone())?,
        ))
    }
    fn advance(&mut self, paths: &Paths) -> Result<()> {
        if self.mode.as_deref() != Some("partial-transcript") || self.phase != 0 {
            return Err(OmmError::invalid("advance requires initial partial stage"));
        }
        let (meeting, _) = self.partial_ids()?;
        let mut store = Store::open(paths)?;
        store.desktop_atomic(
            "fixture_partial_advance",
            &json!({"requestId":Uuid::new_v4(),"payload":{}}),
            |store| {
                commit_segment(store, meeting, 1)?;
                Ok(json!({"advanced":true}))
            },
        )?;
        self.phase = 1;
        Ok(())
    }
    fn finalize(&mut self, paths: &Paths) -> Result<()> {
        if self.mode.as_deref() != Some("partial-transcript") {
            return Ok(());
        }
        if self.phase == 0 {
            self.advance(paths)?;
        }
        if self.phase == 2 {
            return Ok(());
        }
        let (meeting, parent) = self.partial_ids()?;
        let mut store = Store::open(paths)?;
        store.desktop_atomic(
            "fixture_partial_finalize",
            &json!({"requestId":Uuid::new_v4(),"payload":{}}),
            |store| {
                let mut document = store.meeting(meeting)?.unwrap();
                document.sealed_at_ms = chrono::Utc::now().timestamp_millis();
                store.seal_segmented(&document)?;
                let transcript = store.partial_transcript(meeting)?;
                let now = chrono::Utc::now().timestamp_millis();
                let claim = store.claim_asr(now, 30_000)?.ok_or_else(|| {
                    OmmError::unavailable("synthetic aggregate claim unavailable")
                })?;
                if claim.task.id != parent {
                    return Err(OmmError::invalid("refusing another parent claim"));
                }
                store.commit_transcript(&claim, &transcript, now + 1)?;
                Ok(json!({"finalized":true}))
            },
        )?;
        self.phase = 2;
        Ok(())
    }
    fn inspect(&self, paths: &Paths) -> Result<Value> {
        let scenario: Value =
            serde_json::from_slice(&std::fs::read(output("review-scenario.json"))?)?;
        let mut value = json!({"scenario":scenario,"phase":self.phase,"finished":self.finished});
        if self.mode.as_deref() == Some("partial-transcript") {
            let (_, task) = self.partial_ids()?;
            let store = Store::open_read_only(paths)?;
            value["partialRevision"] = json!(store
                .desktop_partial_artifact(task)?
                .map(|(_, revision)| revision));
            value["wholeCommitted"] = json!(store.artifact_committed(task, Stage::Transcribe)?);
        } else {
            value["capturePids"] = json!(capture_pids()?);
            value["captureEvents"] = json!(events()?);
        }
        Ok(value)
    }
    pub fn request(&mut self, paths: &Paths, value: Value) -> Value {
        let id = value["id"]
            .as_str()
            .filter(|s| s.len() <= 64)
            .map(str::to_owned);
        let action = value["action"]
            .as_str()
            .filter(|s| s.len() <= 16)
            .map(str::to_owned);
        let result = (|| -> Result<Value> {
            self.count += 1;
            let request: Request = serde_json::from_value(value)?;
            if self.mode.is_none()
                || self.count > 32
                || self.finished
                || self.expired()
                || !valid(&request)
                || !self.ids.insert(request.id)
            {
                return Err(OmmError::invalid(
                    "invalid/duplicate/exhausted/finished review control",
                ));
            }
            match request.action.as_str() {
                "inspect" => {}
                "advance" => self.advance(paths)?,
                "finalize" => {
                    if self.phase != 1 {
                        return Err(OmmError::invalid(
                            "finalize requires advanced partial stage",
                        ));
                    }
                    self.finalize(paths)?;
                }
                "restore" | "finish" => {
                    self.finalize(paths)?;
                    if self.mode.as_deref() == Some("cold-recording") {
                        let store = Store::open_read_only(paths)?;
                        if store.state::<Value>("recording")?.is_some() {
                            return Err(OmmError::invalid(
                                "stop real recording before restoring helper",
                            ));
                        }
                        let adapter = adapter(paths)?;
                        if capture_socket(paths)?.exists() {
                            tauri::async_runtime::block_on(adapter.shutdown())?;
                        }
                    }
                    self.finished = request.action == "finish";
                }
                _ => unreachable!(),
            };
            self.inspect(paths)
        })();
        let reply = match result {
            Ok(data) => json!({"id":id,"action":action,"ok":true,"data":data}),
            Err(error) => json!({"id":id,"action":action,"ok":false,"error":error.to_string()}),
        };
        if self.receipts.len() < 33 {
            self.receipts.push(reply.clone());
        }
        reply
    }
}
fn capture_socket(paths: &Paths) -> Result<PathBuf> {
    Ok(paths
        .runtime_location(&std::env::vars().collect())?
        .path
        .join("capture/audio_daemon.sock"))
}
fn capture_pids() -> Result<Vec<i32>> {
    validate_private_file(&output("review-capture-pids"))?;
    let bytes = std::fs::read_to_string(output("review-capture-pids"))?;
    if bytes.len() > 4096 {
        return Err(OmmError::invalid("capture pid budget"));
    }
    bytes
        .lines()
        .map(|s| {
            s.parse::<i32>()
                .ok()
                .filter(|p| *p > 0)
                .ok_or_else(|| OmmError::invalid("capture pid receipt invalid"))
        })
        .collect()
}
fn events() -> Result<Vec<Value>> {
    validate_private_file(&output("review-capture-events.jsonl"))?;
    let bytes = std::fs::read_to_string(output("review-capture-events.jsonl"))?;
    if bytes.len() > 64 * 1024 {
        return Err(OmmError::invalid("capture event budget"));
    }
    bytes
        .lines()
        .map(|s| serde_json::from_str(s).map_err(Into::into))
        .collect()
}
fn adapter(paths: &Paths) -> Result<MacCaptureAdapter> {
    let runtime = paths.runtime_location(&std::env::vars().collect())?.path;
    let instance = Store::open_read_only(paths)?
        .state::<Uuid>("capture_instance")?
        .ok_or_else(|| OmmError::unavailable("capture instance missing"))?;
    MacCaptureAdapter::new(
        CapturePaths {
            state: paths.state.join("capture"),
            logs: paths.state.join("logs"),
            media: paths.data.join("meetings"),
            ipc: runtime.join("capture"),
        },
        instance,
        Duration::from_secs(3),
    )
}
pub fn poll(view: &tauri::Webview<tauri::Wry>, paths: Paths) {
    if mode().is_none() {
        return;
    }
    let view = view.clone();
    let owned = view.clone();
    let _=view.eval_with_callback("JSON.stringify((()=>{window.__ommReviewReady=true;const r=window.__ommReviewControl;window.__ommReviewControl=null;return r??null})())",move|text|{
        let Ok(Value::String(text))=serde_json::from_str::<Value>(&text)else{return;};let Ok(value)=serde_json::from_str::<Value>(&text)else{return;};if value.is_null(){return;}
        let owned=owned.clone();let paths=paths.clone();
        std::thread::spawn(move||{if !owned.app_handle().get_webview_window("main").is_some_and(|w|owned.label()=="main"&&crate::host::trusted(&w)){return;}
            let result=owned.app_handle().state::<Mutex<Controller>>().lock().unwrap().request(&paths,value);let _=owned.eval(format!("window.__ommReviewControlResult={result}"));
        });
    });
}
pub fn finish(app: tauri::AppHandle<tauri::Wry>, paths: Paths, mut result: Value) {
    tauri::async_runtime::spawn(async move {
        let report = {
            let state = app.state::<Mutex<Controller>>();
            let mut controller = state.lock().unwrap();
            let cleanup = controller.finalize(&paths);
            json!({"controls":controller.receipts,"explicitFinish":controller.finished,"cleanup":cleanup.is_ok(),"error":cleanup.err().map(|e|e.to_string())})
        };
        let shutdown = omm::service::daemon::request(&paths, "shutdown", None)
            .await
            .is_ok();
        let helper = app
            .state::<Mutex<Option<capture::ChildGuard>>>()
            .lock()
            .unwrap()
            .take();
        let mut prestarted_waited = None;
        if let Some(mut child) = helper {
            for _ in 0..100 {
                if child.0.try_wait().is_ok_and(|s| s.is_some()) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            prestarted_waited = Some(
                child
                    .0
                    .try_wait()
                    .is_ok_and(|s| s.is_some_and(|s| s.success())),
            );
            drop(child);
        }
        let pids = if mode().as_deref() == Some("cold-recording") {
            capture_pids().unwrap_or_default()
        } else {
            vec![]
        };
        let cold_events = if mode().as_deref() == Some("cold-recording") {
            events().unwrap_or_default()
        } else {
            vec![]
        };
        let capture_proof = mode().as_deref() != Some("cold-recording")
            || (!pids.is_empty()
                && pids.iter().all(|pid| {
                    ["bound", "start", "stop", "sealed_segments", "quit"]
                        .iter()
                        .all(|action| {
                            cold_events
                                .iter()
                                .any(|event| event["pid"] == *pid && event["action"] == *action)
                        })
                }));
        let capture_exited=pids.iter().all(|pid|unsafe{libc::kill(*pid,0)}==-1&&std::io::Error::last_os_error().raw_os_error()==Some(libc::ESRCH));
        let readers_before = Store::open_read_only(&paths)
            .and_then(|s| s.active_audio_readers())
            .is_ok_and(|r| !r);
        app.state::<crate::host::Host>().fixture_release_readers();
        let readers = Store::open_read_only(&paths)
            .and_then(|s| s.active_audio_readers())
            .is_ok_and(|r| !r);
        if !readers_before {
            result["ok"] = json!(false);
        }
        result["nativeReview"] = json!({"controller":report,"shutdownAcknowledged":shutdown,"prestartedChildWaited":prestarted_waited,"capturePids":pids,"captureEvents":cold_events,"productionCaptureWaitConfirmed":if mode().as_deref()==Some("cold-recording"){Some(capture_proof&&capture_exited&&shutdown)}else{None},"captureProcessesExited":capture_exited,"captureSocketRemoved":capture_socket(&paths).is_ok_and(|p|!p.exists()),"readersReleased":readers,"readersReleasedBeforeFixtureTeardown":readers_before});
        if report["cleanup"] != true
            || report["explicitFinish"] != true
            || !shutdown
            || prestarted_waited == Some(false)
            || !capture_proof
            || !capture_exited
            || !readers
        {
            result["ok"] = json!(false);
        }
        write_private_new(
            &output("bridge-result.json"),
            &serde_json::to_vec_pretty(
                &json!({"result":result,"profile":"private review producers"}),
            )
            .unwrap(),
        )
        .unwrap();
        app.exit(if result["ok"] == true { 0 } else { 1 });
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn review_control_rejects_paths_unknown_actions_and_unbounded_ids() {
        for value in [
            json!({"id":"x","action":"advance","path":"/tmp"}),
            json!({"id":"../x","action":"restore"}),
            json!({"id":"x","action":"sql"}),
            json!({"id":"x".repeat(65),"action":"finish"}),
        ] {
            assert!(!serde_json::from_value::<Request>(value).is_ok_and(|r| valid(&r)));
        }
        assert!(valid(&Request {
            id: "root-1".into(),
            action: "inspect".into()
        }));
    }
}

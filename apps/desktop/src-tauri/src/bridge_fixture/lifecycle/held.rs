//! Bounded, reversible negative states for a trusted private lifecycle UI driver only.
use super::*;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    process::Stdio,
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: String,
    action: String,
}

enum Held {
    Busy {
        session: Uuid,
        directory: PathBuf,
    },
    Unknown {
        child: capture::ChildGuard,
    },
    Stale {
        original: Vec<u8>,
        replacement: Vec<u8>,
    },
}
impl Held {
    fn name(&self) -> &'static str {
        match self {
            Self::Busy { .. } => "busy",
            Self::Unknown { .. } => "unknown",
            Self::Stale { .. } => "stale",
        }
    }
}
#[derive(Default)]
pub(super) struct Controller {
    ids: BTreeSet<String>,
    attempts: usize,
    held: Option<Held>,
    deadline: Option<Instant>,
    finished: bool,
    case_running: bool,
    receipts: Vec<Value>,
    cleanup_failed: bool,
}
fn valid(request: &Request) -> bool {
    !request.id.is_empty()
        && request.id.len() <= 64
        && request
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && [
            "hold-busy",
            "hold-unknown",
            "stale-plan",
            "restore",
            "finish",
        ]
        .contains(&request.action.as_str())
}
impl Controller {
    pub fn begin_case(&mut self) -> bool {
        if self.active() || self.case_running {
            return false;
        }
        self.case_running = true;
        true
    }
    pub fn end_case(&mut self) {
        self.case_running = false;
    }
    pub fn active(&self) -> bool {
        self.held.is_some()
    }
    pub fn expired(&self) -> bool {
        self.deadline.is_some_and(|d| Instant::now() >= d)
    }
    pub fn restore(&mut self, paths: &Paths) -> std::result::Result<Value, String> {
        self.deadline = None;
        let Some(mut held) = self.held.take() else {
            return Ok(json!({"restored":null}));
        };
        let name = held.name();
        let result = (|| -> std::result::Result<Value, String> {
            match &mut held {
                Held::Busy { session, directory } => {
                    let store = Store::open(paths).map_err(|e| e.to_string())?;
                    let record = store
                        .state::<Value>("recording")
                        .map_err(|e| e.to_string())?;
                    if record
                        .as_ref()
                        .is_none_or(|r| r["session_id"] != session.to_string())
                    {
                        return Err(
                            "synthetic recording identity changed; not clearing foreign state"
                                .into(),
                        );
                    }
                    store.clear_state("recording").map_err(|e| e.to_string())?;
                    std::fs::remove_dir(directory).map_err(|e| e.to_string())?;
                    Ok(json!({"restored":name,"recordingCleared":true,"sessionId":session}))
                }
                Held::Stale {
                    original,
                    replacement,
                } => {
                    let _offline = profile::offline_lock(paths).map_err(|e| e.to_string())?;
                    let _writer = profile::writer_lock(paths).map_err(|e| e.to_string())?;
                    let owner = paths.state.join("desktop-owner.json");
                    if std::fs::read(&owner).map_err(|e| e.to_string())?.as_slice()
                        != replacement.as_slice()
                    {
                        return Err("held generation changed; not overwriting foreign owner".into());
                    }
                    write_private_replace(&owner, original).map_err(|e| e.to_string())?;
                    Ok(json!({"restored":name,"originalOwnerRestored":true}))
                }
                Held::Unknown { child } => {
                    if let Some(mut input) = child.0.stdin.take() {
                        let _ = input.write_all(b"q");
                    }
                    let mut status = None;
                    for _ in 0..200 {
                        status = child.0.try_wait().map_err(|e| e.to_string())?;
                        if status.is_some() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    if status.is_none() {
                        return Err("owned unknown listener still live; handle and held state retained without signals".into());
                    }
                    let status = child.0.wait().map_err(|e| e.to_string())?;
                    let location = paths
                        .runtime_location(&std::env::vars().collect())
                        .map_err(|e| e.to_string())?;
                    let released = profile::offline_lock(paths).is_ok()
                        && !location.path.join("service.sock").exists();
                    let receipt = json!({"restored":name,"pid":child.0.id(),"waited":true,"exitCode":status.code(),"forced":false,"socketRemovedAndLocksReleased":released});
                    if !status.success() || !released {
                        self.receipts.push(receipt);
                        return Err("owned unknown listener failed graceful cleanup".into());
                    }
                    Ok(receipt)
                }
            }
        })();
        if result.is_err() {
            self.held = Some(held);
            self.cleanup_failed = true;
        }
        result
    }
    pub fn report(&self) -> Value {
        json!({"requests":self.receipts,"finished":self.finished,"clean":self.held.is_none()&&!self.cleanup_failed,"used":self.attempts})
    }
    pub fn request(
        &mut self,
        app: &tauri::AppHandle<tauri::Wry>,
        paths: &Paths,
        value: Value,
    ) -> Value {
        self.attempts += 1;
        let id = value["id"]
            .as_str()
            .filter(|s| s.len() <= 64)
            .map(str::to_owned);
        let action = value["action"]
            .as_str()
            .filter(|s| s.len() <= 32)
            .map(str::to_owned);
        let result = match serde_json::from_value::<Request>(value) {
            Ok(request) => self.execute(app, paths, request),
            Err(error) => Err(error.to_string()),
        };
        let reply = match result {
            Ok(details) => {
                json!({"id":id,"action":action,"ok":true,"held":self.held.as_ref().map(Held::name),"details":details})
            }
            Err(error) => {
                let restored = self.restore(paths);
                json!({"id":id,"action":action,"ok":false,"held":self.held.as_ref().map(Held::name),"error":error,"cleanup":restored.ok()})
            }
        };
        self.receipts.push(reply.clone());
        reply
    }
    fn execute(
        &mut self,
        app: &tauri::AppHandle<tauri::Wry>,
        paths: &Paths,
        r: Request,
    ) -> std::result::Result<Value, String> {
        if self.case_running {
            return Err("wait for the current native case before control".into());
        }
        if self.attempts > 32 || self.finished || !valid(&r) || !self.ids.insert(r.id.clone()) {
            return Err("invalid, duplicate, exhausted or finished lifecycle control".into());
        }
        let case = app
            .state::<Mutex<Vec<Value>>>()
            .lock()
            .unwrap()
            .last()
            .and_then(|c| c["case"].as_str())
            .unwrap_or("")
            .to_owned();
        if r.action == "restore" || r.action == "finish" {
            if r.action == "restore" && self.held.is_none() {
                return Err("no held state to restore".into());
            }
            if r.action == "finish" && case != "offline" {
                return Err("finish requires completed offline case; publish failed fixture result for earlier abort".into());
            }
            let mut restored = self.restore(paths)?;
            if r.action == "finish" {
                let _offline = profile::offline_lock(paths).map_err(|e| e.to_string())?;
                let helper = app.state::<Mutex<Option<capture::ChildGuard>>>();
                if let Some(helper) = helper.lock().unwrap().as_mut() {
                    if helper.0.try_wait().map_err(|e| e.to_string())?.is_none() {
                        return Err("capture peer not sealed/stopped".into());
                    }
                    let status = helper.0.wait().map_err(|e| e.to_string())?;
                    restored["capturePeerWaited"] =
                        json!({"pid":helper.0.id(),"exitCode":status.code(),"waited":true});
                    if !status.success() {
                        return Err("capture peer failed".into());
                    }
                };
            }
            self.finished = r.action == "finish";
            return Ok(restored);
        }
        if self.held.is_some() {
            return Err("restore held state before another hold".into());
        }
        let bundle = app.state::<Bundle>().inner().clone();
        let location = paths
            .runtime_location(&std::env::vars().collect())
            .map_err(|e| e.to_string())?;
        if !location
            .path
            .starts_with(super::super::output("driver.js").parent().unwrap())
        {
            return Err("runtime must belong to this private lifecycle root".into());
        }
        let held = match r.action.as_str() {
            "hold-busy" => {
                if case != "running"
                    || !runtime::verify_active(&location.path, &paths.state)
                        .is_ok_and(|o| cli_matches(o.pid, &bundle))
                {
                    return Err(
                        "hold-busy requires completed running case and live owned service".into(),
                    );
                }
                let store = Store::open(paths).map_err(|e| e.to_string())?;
                if store
                    .state::<Value>("recording")
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    return Err("refusing to replace existing recording".into());
                }
                let session = Uuid::new_v4();
                let directory = paths.data.join(format!("held-unsealed-{session}"));
                private_dir(&directory).map_err(|e| e.to_string())?;
                let now = chrono::Utc::now().timestamp_millis();
                let record = omm::service::daemon::Recording {
                    session_id: session,
                    title: "Private held unsealed sentinel".into(),
                    directory: directory.clone(),
                    source_id: None,
                    started_at_ms: now,
                    segment_started_ms: now,
                    segment_index: 0,
                    segment_ms: 60_000,
                    config: AppConfig::load(paths)
                        .map_err(|e| e.to_string())?
                        .snapshot(),
                    gaps: vec![],
                    start_confirmed: true,
                    segments: vec![],
                    rotation_pending: false,
                    sealed: None,
                };
                store
                    .set_state("recording", &record)
                    .map_err(|e| e.to_string())?;
                Held::Busy { session, directory }
            }
            "hold-unknown" => {
                if !["configure", "offline"].contains(&case.as_str()) {
                    return Err("unknown requires completed configure or offline case".into());
                }
                let offline = profile::offline_lock(paths).map_err(|e| e.to_string())?;
                if location.path.join("service.sock").exists() {
                    return Err("refusing to replace an existing socket".into());
                }
                drop(offline);
                let mut child = capture::ChildGuard::new(
                    std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                        .env("OMM_LIFECYCLE_UNKNOWN_LISTENER", "1")
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::inherit())
                        .spawn()
                        .map_err(|e| e.to_string())?,
                );
                let mut ready = false;
                for _ in 0..200 {
                    if runtime::verify_active(&location.path, &paths.state)
                        .is_ok_and(|o| o.pid == child.0.id() as i32)
                    {
                        ready = true;
                        break;
                    }
                    if child.0.try_wait().map_err(|e| e.to_string())?.is_some() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                if !ready {
                    self.held = Some(Held::Unknown { child });
                    return Err(
                        "owned unknown listener not ready; held handle retained for normal restore"
                            .into(),
                    );
                }
                Held::Unknown { child }
            }
            "stale-plan" => {
                if !["configure", "offline"].contains(&case.as_str()) {
                    return Err("stale requires completed configure or offline case".into());
                }
                let _offline = profile::offline_lock(paths).map_err(|e| e.to_string())?;
                let _writer = profile::writer_lock(paths).map_err(|e| e.to_string())?;
                let owner_path = paths.state.join("desktop-owner.json");
                let original = std::fs::read(&owner_path).map_err(|e| e.to_string())?;
                let mut owner = profile::read(paths)
                    .map_err(|e| e.to_string())?
                    .ok_or("no initialized owner")?;
                owner.generation = Uuid::new_v4();
                let replacement = serde_json::to_vec(&owner).map_err(|e| e.to_string())?;
                write_private_replace(&owner_path, &replacement).map_err(|e| e.to_string())?;
                Held::Stale {
                    original,
                    replacement,
                }
            }
            _ => unreachable!(),
        };
        let name = held.name();
        self.held = Some(held);
        self.deadline = Some(Instant::now() + Duration::from_secs(90));
        // No samples/transport substitution: normal product plans and applies read the held state.
        let action = if name == "busy" { "stop" } else { "start" };
        let plan = tauri::async_runtime::block_on(lifecycle::plan(paths, Some(&bundle), action))
            .map_err(|e| e.to_string())?
            .0;
        Ok(
            json!({"held":name,"expiresAtMs":chrono::Utc::now().timestamp_millis()+90_000,"observedPlan":plan,"listenerPid":match self.held.as_ref(){Some(Held::Unknown{child})=>Some(child.0.id()),_=>None}}),
        )
    }
}
/// Independent fixture executable, never the manifest-bound CLI. Parent pipe closes on failure.
pub(crate) fn run_child() {
    validate_private_file(&super::super::output("driver.js")).unwrap();
    assert_eq!(
        std::env::var("OMM_WKWEBVIEW_SCENARIO").unwrap(),
        "lifecycle"
    );
    let paths = Paths::from_env().unwrap();
    assert!(paths
        .state
        .starts_with(super::super::output("driver.js").parent().unwrap()));
    let location = paths.runtime_location(&std::env::vars().collect()).unwrap();
    assert!(location
        .path
        .starts_with(super::super::output("driver.js").parent().unwrap()));
    let guard = RuntimeGuard::acquire(&location, &paths.state).unwrap();
    let listener =
        std::os::unix::net::UnixListener::bind(location.path.join("service.sock")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        location.path.join("service.sock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let input = stop.clone();
    std::thread::spawn(move || {
        let _ = std::io::stdin().read(&mut [0u8]);
        input.store(true, Ordering::SeqCst);
    });
    let deadline = Instant::now() + Duration::from_secs(110);
    while !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => drop(stream),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) => panic!("private listener: {e}"),
        }
    }
    drop(listener);
    drop(guard);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_control_is_strict_and_bounded() {
        for value in [
            json!({"id":"x","action":"hold-busy","path":"/tmp"}),
            json!({"id":"x","action":"sql"}),
            json!({"id":"../x","action":"restore"}),
            json!({"id":"x"}),
        ] {
            assert!(serde_json::from_value::<Request>(value).map_or(true, |r| !valid(&r)));
        }
        assert!(valid(&Request {
            id: "root-1".into(),
            action: "hold-busy".into()
        }));
        assert!(!valid(&Request {
            id: "x".repeat(65),
            action: "finish".into()
        }));
    }
    #[test]
    fn lifecycle_expired_synthetic_state_restores_without_touching_other_data() {
        let root = PathBuf::from("/private/tmp").join(format!("omm-held-unit-{}", Uuid::new_v4()));
        private_dir(&root).unwrap();
        let paths = Paths::resolve(&root.join("home"), &BTreeMap::new()).unwrap();
        paths.create_private_dirs().unwrap();
        let store = Store::open(&paths).unwrap();
        let session = Uuid::new_v4();
        let directory = paths.data.join("private-held");
        private_dir(&directory).unwrap();
        store
            .set_state("recording", &json!({"session_id":session,"sealed":null}))
            .unwrap();
        store
            .set_state("unrelated-fixture", &json!({"retained":true}))
            .unwrap();
        let mut controller = Controller {
            held: Some(Held::Busy {
                session,
                directory: directory.clone(),
            }),
            deadline: Some(Instant::now() - Duration::from_secs(1)),
            ..Default::default()
        };
        assert!(controller.expired());
        assert!(!controller.begin_case());
        assert_eq!(
            controller.restore(&paths).unwrap()["recordingCleared"],
            true
        );
        assert!(!controller.expired());
        assert!(controller.begin_case());
        controller.end_case();
        assert!(store.state::<Value>("recording").unwrap().is_none());
        assert_eq!(
            store.state::<Value>("unrelated-fixture").unwrap().unwrap()["retained"],
            true
        );
        assert!(!directory.exists());
        let owner = profile::ProfileOwner {
            schema_version: 1,
            min_reader_version: 1,
            generation: Uuid::new_v4(),
            profile_id: profile::profile_id(&paths).unwrap(),
        };
        let original = serde_json::to_vec(&owner).unwrap();
        let mut replacement_owner = owner.clone();
        replacement_owner.generation = Uuid::new_v4();
        let replacement = serde_json::to_vec(&replacement_owner).unwrap();
        write_private_new(&paths.state.join("desktop-owner.json"), &replacement).unwrap();
        controller.held = Some(Held::Stale {
            original: original.clone(),
            replacement,
        });
        assert_eq!(
            controller.restore(&paths).unwrap()["originalOwnerRestored"],
            true
        );
        assert_eq!(
            std::fs::read(paths.state.join("desktop-owner.json")).unwrap(),
            original
        );
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}

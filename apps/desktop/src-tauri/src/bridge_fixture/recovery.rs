//! Feature-only system-manager boundary for real React recovery; never falls back to launchctl.
use super::*;
use async_trait::async_trait;
use omm::{
    application::{bundle::Bundle, lifecycle},
    core::runtime::{self, RuntimeGuard},
    service::registration::{self, ServiceRunner},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    sync::{atomic::AtomicUsize, Mutex, OnceLock},
};

struct Fixture {
    paths: Paths,
    root: PathBuf,
    home: PathBuf,
    nonce: Uuid,
    pid: u32,
    ready: AtomicBool,
    finishing: AtomicBool,
    loaded: AtomicBool,
    queries: AtomicUsize,
    unexpected: AtomicUsize,
    original_config: Mutex<Option<Vec<u8>>>,
    owners: Mutex<Vec<runtime::RuntimeOwner>>,
}
static FIXTURE: OnceLock<Arc<Fixture>> = OnceLock::new();
fn denied() -> OmmError {
    OmmError::invalid("private recovery fixture admission refused")
}
fn validate(f: &Fixture, paths: &Paths) -> Result<()> {
    if f.pid != std::process::id()
        || !f.ready.load(Ordering::SeqCst)
        || serde_json::to_value(&f.paths)? != serde_json::to_value(paths)?
    {
        return Err(denied());
    }
    for path in [
        &f.root,
        &f.home,
        &paths.config,
        &paths.data,
        &paths.state,
        &paths.cache,
    ] {
        validate_private_dir(path)?;
        if path.canonicalize()? != *path || !path.starts_with(&f.root) {
            return Err(denied());
        }
    }
    if std::env::var_os("HOME").ok_or_else(denied)? != f.home {
        return Err(denied());
    }
    if Paths::from_env()? != f.paths {
        return Err(denied());
    }
    let runtime = paths.runtime_location(&std::env::vars().collect())?;
    validate_private_dir(&runtime.path)?;
    if runtime.path.canonicalize()? != runtime.path || !runtime.path.starts_with(&f.root) {
        return Err(denied());
    }
    let marker = f.root.join("recovery-admission.json");
    validate_private_file(&marker)?;
    let actual: Value = serde_json::from_slice(&fs::read(marker)?)?;
    if actual != json!({"nonce":f.nonce,"pid":f.pid,"root":f.root}) {
        return Err(denied());
    }
    Ok(())
}
fn admitted(paths: &Paths) -> Result<Option<Arc<Fixture>>> {
    if scenario() != "recovery" {
        return Ok(None);
    }
    let f = FIXTURE.get().ok_or_else(denied)?.clone();
    validate(&f, paths)?;
    Ok(Some(f))
}
#[async_trait]
impl ServiceRunner for Fixture {
    async fn require_unloaded(&self) -> Result<()> {
        validate(self, &self.paths)?;
        self.queries.fetch_add(1, Ordering::SeqCst);
        if self.loaded.load(Ordering::SeqCst) {
            Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                registration::recovery::EXTERNAL_UNLOAD_REQUIRED,
            ))
        } else {
            Ok(())
        }
    }
    async fn run(&self, _args: &[String]) -> Result<()> {
        self.unexpected.fetch_add(1, Ordering::SeqCst);
        Err(OmmError::invalid(
            "unexpected fake system-manager control; no real fallback",
        ))
    }
}
pub async fn plan(
    paths: &Paths,
    bundle: Option<&Bundle>,
    action: &str,
) -> Result<(Value, Option<lifecycle::Plan>)> {
    match admitted(paths)? {
        Some(f) => {
            lifecycle::plan_at(
                paths,
                bundle,
                action,
                &f.home,
                &paths.runtime_location(&std::env::vars().collect())?,
                f.as_ref(),
            )
            .await
        }
        None => lifecycle::plan(paths, bundle, action).await,
    }
}
pub async fn apply(paths: &Paths, bundle: &Bundle, plan: lifecycle::Plan) -> Result<Value> {
    match admitted(paths)? {
        Some(f) => {
            lifecycle::apply_at(
                paths,
                bundle,
                plan,
                &f.home,
                &paths.runtime_location(&std::env::vars().collect())?,
                f.as_ref(),
            )
            .await
        }
        None => lifecycle::apply(paths, bundle, plan).await,
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Control {
    nonce: Uuid,
    id: String,
    action: String,
}
fn control(f: &Fixture, value: Value) -> Result<Value> {
    validate(f, &f.paths)?;
    let request: Control = serde_json::from_value(value)?;
    if request.nonce != f.nonce || request.id.len() > 64 {
        return Err(denied());
    }
    let owner = f.paths.state.join("desktop-owner.json");
    match request.action.as_str() {
        "unloaded" => {
            f.loaded.store(false, Ordering::SeqCst);
        }
        "restore-config" => {
            let bytes = f
                .original_config
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(denied)?;
            write_private_replace(&f.paths.config.join("config.json"), &bytes)?;
        }
        "change-generation" => {
            let mut value: Value = serde_json::from_slice(&fs::read(&owner)?)?;
            value["generation"] = json!(Uuid::new_v4());
            write_private_replace(&owner, &serde_json::to_vec(&value)?)?;
        }
        _ => return Err(denied()),
    }
    Ok(json!({"ok":true,"id":request.id,"action":request.action}))
}
pub fn builder(builder: tauri::Builder<tauri::Wry>, paths: &Paths) -> tauri::Builder<tauri::Wry> {
    let root = output("driver.js").parent().unwrap().to_path_buf();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap())
        .canonicalize()
        .unwrap();
    assert!(home.starts_with(&root));
    paths.create_private_dirs().unwrap();
    let mut cfg = AppConfig {
        auto_record: false,
        recording_consent: false,
        ..AppConfig::default()
    };
    cfg.notifications.enabled = false;
    cfg.save(paths).unwrap();
    drop(Store::open(paths).unwrap());
    let runtime = paths.runtime_location(&std::env::vars().collect()).unwrap();
    drop(RuntimeGuard::acquire(&runtime, &paths.state).unwrap());
    let old = root.join("old-cli-never-executed");
    write_private_new(&old, b"#!/bin/sh\nexit 99\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&old, fs::Permissions::from_mode(0o700)).unwrap();
    let f = Arc::new(Fixture {
        paths: paths.clone(),
        root,
        home,
        nonce: Uuid::new_v4(),
        pid: std::process::id(),
        ready: AtomicBool::new(false),
        finishing: AtomicBool::new(false),
        loaded: AtomicBool::new(true),
        queries: AtomicUsize::new(0),
        unexpected: AtomicUsize::new(0),
        original_config: Mutex::new(None),
        owners: Mutex::new(vec![]),
    });
    // Installation only renders private metadata; every system operation is rejected by this runner.
    struct Install {
        expected: Vec<String>,
    }
    #[async_trait]
    impl ServiceRunner for Install {
        async fn run(&self, args: &[String]) -> Result<()> {
            if args != self.expected {
                return Err(denied());
            }
            Ok(())
        }
    }
    let install = Install {
        expected: vec![
            "bootstrap".into(),
            format!("gui/{}", unsafe { libc::geteuid() }),
            f.home
                .join("Library/LaunchAgents/org.openmeetingmemo.service.plist")
                .to_str()
                .unwrap()
                .into(),
        ],
    };
    tauri::async_runtime::block_on(registration::install_at(
        &f.home,
        paths,
        &old,
        Some(&runtime),
        &install,
    ))
    .unwrap();
    let mut cfg = AppConfig::load(paths).unwrap();
    cfg.notifications.enabled = false;
    cfg.capture_executable = std::env::current_exe().unwrap();
    cfg.poll_ms = 60_000;
    cfg.save(paths).unwrap();
    *f.original_config.lock().unwrap() = Some(fs::read(paths.config.join("config.json")).unwrap());
    let mut missing: Value =
        serde_json::from_slice(f.original_config.lock().unwrap().as_ref().unwrap()).unwrap();
    missing.as_object_mut().unwrap().remove("auto_record");
    missing.as_object_mut().unwrap().remove("recording_consent");
    write_private_replace(
        &paths.config.join("config.json"),
        &serde_json::to_vec(&missing).unwrap(),
    )
    .unwrap();
    write_private_new(
        &f.root.join("recovery-admission.json"),
        &serde_json::to_vec(&json!({"nonce":f.nonce,"pid":f.pid,"root":f.root})).unwrap(),
    )
    .unwrap();
    assert!(FIXTURE.set(f.clone()).is_ok());
    let started = Arc::new(AtomicBool::new(false));
    builder.on_page_load(move |view, event| {
        if event.event() != tauri::webview::PageLoadEvent::Finished || view.label() != "main" || !view.app_handle().get_webview_window("main").is_some_and(|w| crate::host::trusted(&w)) || started.swap(true, Ordering::SeqCst) { return; }
        f.ready.store(true, Ordering::SeqCst);
        view.eval(format!("window.__ommRecoveryNonce={};{}", json!(f.nonce), fs::read_to_string(output("driver.js")).unwrap())).unwrap();
        let view = view.clone(); let f = f.clone();
        std::thread::spawn(move || {
            for _ in 0..600 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if let Ok(runtime) = f.paths.runtime_location(&std::env::vars().collect()) {
                    if let Ok(owner) = runtime::verify_active(&runtime.path, &f.paths.state) {
                        if Bundle::locate().is_ok_and(|b| lifecycle::owned_service(&owner, &b).unwrap_or(false)) {
                            let mut owners = f.owners.lock().unwrap();
                            if !owners.iter().any(|v| v.instance_id == owner.instance_id) { owners.push(owner); }
                        }
                    }
                }
                snapshot::poll(&view);
                let f = f.clone(); let owned = view.clone();
                view.eval_with_callback("JSON.stringify({control:(()=>{const v=window.__ommRecoveryControl;window.__ommRecoveryControl=null;return v??null})(),result:window.__ommFixtureResult??null})", move |text| {
                    let Ok(Value::String(text)) = serde_json::from_str::<Value>(&text) else { return; };
                    let Ok(value) = serde_json::from_str::<Value>(&text) else { return; };
                    if !owned.app_handle().get_webview_window("main").is_some_and(|w| crate::host::trusted(&w)) { return; }
                    if value["control"].is_object() {
                        let result = control(&f, value["control"].clone()).unwrap_or_else(|e| json!({"ok":false,"error":e.message,"id":value["control"]["id"]}));
                        let _ = owned.eval(format!("window.__ommRecoveryControlResult={result}"));
                    }
                    if value["result"].is_object() && !f.finishing.swap(true, Ordering::SeqCst) {
                        let mut result = value["result"].clone();
                        result["fakeSystemManager"] = json!({"queries":f.queries.load(Ordering::SeqCst),"unexpectedControls":f.unexpected.load(Ordering::SeqCst),"realSystemCalls":0,"nonceBound":true,"pid":f.pid});
                        if f.unexpected.load(Ordering::SeqCst) != 0 { result["ok"] = json!(false); }
                        let f = f.clone();
                        let owned = owned.clone();
                        std::thread::spawn(move || {
                            let runtime = f.paths.runtime_location(&std::env::vars().collect()).unwrap();
                            if let Ok(owner) = runtime::verify_active(&runtime.path, &f.paths.state) {
                                if Bundle::locate().is_ok_and(|b| lifecycle::owned_service(&owner, &b).unwrap_or(false)) {
                                    let _ = tauri::async_runtime::block_on(omm::service::daemon::request_message_at(&f.paths, &runtime, omm::service::daemon::Request {schema_version:1, command:"desktop_idle_shutdown_v1".into(), title:None, session_id:None, instance_id:Some(owner.instance_id), desktop_request:None, desktop_capability:None}));
                                }
                            }
                            for _ in 0..100 {
                                if f.owners.lock().unwrap().iter().all(|o| !runtime::owner_is_live(o)) && omm::application::profile::offline_lock(&f.paths).is_ok() { break; }
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                            let clean = f.owners.lock().unwrap().iter().all(|o| !runtime::owner_is_live(o)) && omm::application::profile::offline_lock(&f.paths).is_ok() && !runtime.path.join("service.sock").exists();
                            result["ownedCleanup"] = json!({"allExitedAndLocksReleased":clean,"owners":*f.owners.lock().unwrap(),"forcedSignals":0});
                            if !clean { result["ok"] = json!(false); }
                            write_private_new(&output("bridge-result.json"), &serde_json::to_vec_pretty(&json!({"result":result})).unwrap()).unwrap();
                            owned.app_handle().exit(if result["ok"] == true {0} else {1});
                        });
                    }
                }).unwrap();
            }
        });
    })
}

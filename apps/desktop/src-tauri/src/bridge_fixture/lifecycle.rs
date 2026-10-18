//! Real bundled CLI lifecycle proofs, only for a fresh explicit private fixture profile.
use super::*;
mod held;
pub(super) fn run_unknown_listener() {
    held::run_child();
}
use omm::{
    application::{bundle::Bundle, lifecycle, profile},
    core::runtime::{self, RuntimeGuard},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Mutex};

fn trusted(view: &tauri::Webview) -> bool {
    view.label() == "main"
        && view
            .app_handle()
            .get_webview_window("main")
            .is_some_and(|w| crate::host::trusted(&w))
}
pub fn builder(builder: tauri::Builder<tauri::Wry>, paths: &Paths) -> tauri::Builder<tauri::Wry> {
    children::core_pending(true);
    let paths = paths.clone();
    let started = Arc::new(AtomicBool::new(false));
    builder.manage(Mutex::new(held::Controller::default())).manage(Bundle::locate().unwrap()).manage(Mutex::<Vec<Value>>::new(vec![])).manage(Mutex::<Option<capture::ChildGuard>>::new(None)).on_page_load(move |view, event| {
        if !trusted(view) || view.label() != "main" || event.event() != tauri::webview::PageLoadEvent::Finished || started.swap(true, Ordering::SeqCst) { return; }
        let driver = std::fs::read_to_string(output("driver.js")).unwrap();
        view.eval(driver).unwrap();
        let view = view.clone();
        let paths = paths.clone();
        std::thread::spawn(move || {
            let seen = Arc::new(Mutex::new(BTreeMap::new()));
            let cases = Arc::new(Mutex::new(Vec::<String>::new()));
            let done = Arc::new(AtomicBool::new(false));
            for _ in 0..720 {
                if done.load(Ordering::SeqCst) { return; }
                std::thread::sleep(std::time::Duration::from_millis(500));
                let paths = paths.clone(); let seen = seen.clone(); let cases = cases.clone(); let done = done.clone(); let owned = view.clone();
                if view.app_handle().state::<Mutex<held::Controller>>().lock().unwrap().expired() && !done.swap(true, Ordering::SeqCst) {
                    finish(view.app_handle().clone(), paths, json!({"ok":false,"playCalls":0,"error":"held lifecycle stage exceeded 90 seconds"}), &seen); return;
                }
                if !trusted(&view) { continue; }
                view.eval_with_callback("JSON.stringify({debug:window.__ommLifecycleDebug||null,request:window.__ommLifecycleCase||null,control:(()=>{const r=window.__ommLifecycleControl;window.__ommLifecycleControl=null;return r??null})(),result:window.__ommFixtureResult||null})", move |text| {
                    let Ok(Value::String(text)) = serde_json::from_str::<Value>(&text) else { return; };
                    let Ok(value) = serde_json::from_str::<Value>(&text) else { return; };
                    let owned=owned.clone(); let paths=paths.clone(); let seen=seen.clone(); let cases=cases.clone(); let done=done.clone();
                    std::thread::spawn(move || {
                    if !trusted(&owned) { return; }
                    eprintln!("private lifecycle checkpoint {}", value["debug"]);
                    if value["result"].is_object() && !done.swap(true, Ordering::SeqCst) {
                        let result = value["result"].clone(); let owned = owned.clone(); let paths = paths.clone(); let seen = seen.clone();
                        std::thread::spawn(move || finish(owned.app_handle().clone(), paths, result, &seen));
                    } else if !value["control"].is_null() {
                        let result = owned.app_handle().state::<Mutex<held::Controller>>().lock().unwrap().request(owned.app_handle(), &paths, value["control"].clone());
                        if trusted(&owned) { let _ = owned.eval(format!("window.__ommLifecycleControlResult={result}")); }
                    } else if let Some(case) = value["request"].as_str() {
                        let case = case.to_string(); let mut used = cases.lock().unwrap();
                        if !["configure", "running", "offline", "switched"].contains(&case.as_str()) || used.contains(&case) { return; }
                        used.push(case.clone()); drop(used);
                        let owned = owned.clone(); let paths = paths.clone(); let seen = seen.clone();
                        std::thread::spawn(move || {
                            let admitted = owned.app_handle().state::<Mutex<held::Controller>>().lock().unwrap().begin_case();
                            let result = if !admitted { Ok(json!({"ok":false,"case":case,"error":"restore held state before advancing case"})) } else { std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tauri::async_runtime::block_on(cases_run(owned.app_handle(), &paths, &case, &seen)))) };
                            if admitted { owned.app_handle().state::<Mutex<held::Controller>>().lock().unwrap().end_case(); }
                            let result = result.unwrap_or_else(|_| json!({"ok":false,"case":case}));
                            owned.app_handle().state::<Mutex<Vec<Value>>>().lock().unwrap().push(result.clone());
                            let _ = owned.eval(format!("window.__ommLifecycleCase=null;window.__ommLifecycleCaseResult={result}"));
                        });
                    }
                    });
                }).unwrap();
            }
            if !done.swap(true, Ordering::SeqCst) { finish(view.app_handle().clone(), paths, json!({"ok":false,"playCalls":0,"error":"lifecycle fixture exceeded 360 seconds"}), &seen); }
        });
    })
}
async fn cases_run(
    app: &tauri::AppHandle<tauri::Wry>,
    paths: &Paths,
    case: &str,
    seen: &Mutex<BTreeMap<i32, runtime::RuntimeOwner>>,
) -> Value {
    let bundle = app.state::<Bundle>().inner().clone();
    let location = paths.runtime_location(&std::env::vars().collect()).unwrap();
    if case == "configure" {
        let mut cfg = AppConfig::load(paths).unwrap();
        cfg.capture_executable = std::env::current_exe().unwrap();
        cfg.poll_ms = 60_000;
        cfg.save(paths).unwrap();
        return json!({"ok":true,"case":case,"capture":"private synthetic peer only"});
    }
    if case == "running" || case == "switched" {
        let owner = runtime::verify_active(&location.path, &paths.state).unwrap();
        assert!(cli_matches(owner.pid, &bundle));
        seen.lock().unwrap().insert(owner.pid, owner.clone());
        assert!(!AppConfig::load(paths).unwrap().auto_record);
        assert!(!AppConfig::load(paths).unwrap().notifications.enabled);
        if case == "running" {
            let (_, plan) = lifecycle::plan(paths, Some(&bundle), "stop").await.unwrap();
            let store = Store::open(paths).unwrap();
            // Only the fixture peer can be observed/launched: the configuration never
            // points at real capture for this negative persisted-recording proof.
            let helper = capture::start(paths);
            let directory = paths.data.join("synthetic-unsealed");
            private_dir(&directory).unwrap();
            let now = chrono::Utc::now().timestamp_millis();
            let record = omm::service::daemon::Recording {
                session_id: Uuid::new_v4(),
                title: "Private unsealed sentinel".into(),
                directory,
                source_id: None,
                started_at_ms: now,
                segment_started_ms: now,
                segment_index: 0,
                segment_ms: 60_000,
                config: AppConfig::load(paths).unwrap().snapshot(),
                gaps: vec![],
                start_confirmed: true,
                segments: vec![],
                rotation_pending: false,
                sealed: None,
            };
            store.set_state("recording", &record).unwrap();
            let (busy, _) = lifecycle::plan(paths, Some(&bundle), "stop").await.unwrap();
            let rejected = lifecycle::apply(paths, &bundle, plan.unwrap())
                .await
                .is_err();
            store.clear_state("recording").unwrap();
            *app.state::<Mutex<Option<capture::ChildGuard>>>()
                .lock()
                .unwrap() = Some(helper);
            assert_eq!(busy["state"], "busy");
            assert_eq!(busy["canApply"], false);
            assert!(rejected);
            write_private_new(
                &paths.data.join("lifecycle-preserve.txt"),
                b"private synthetic data must survive start-stop-switch",
            )
            .unwrap();
        }
        return json!({"ok":true,"case":case,"ownedPid":owner.pid,"executable":bundle.cli().unwrap(),"actualExecutableVerified":true,"recordingUnsealedRefused":case=="running"});
    }
    assert!(profile::offline_lock(paths).is_ok());
    let (_, plan) = lifecycle::plan(paths, Some(&bundle), "start")
        .await
        .unwrap();
    let mut expired = plan.unwrap();
    expired.expires = chrono::Utc::now().timestamp_millis() - 1;
    assert!(lifecycle::apply(paths, &bundle, expired).await.is_err());
    let (_, plan) = lifecycle::plan(paths, Some(&bundle), "start")
        .await
        .unwrap();
    let owner_path = paths.state.join("desktop-owner.json");
    let original = std::fs::read(&owner_path).unwrap();
    let mut owner = profile::read(paths).unwrap().unwrap();
    owner.generation = Uuid::new_v4();
    write_private_replace(&owner_path, &serde_json::to_vec(&owner).unwrap()).unwrap();
    let stale_rejected = lifecycle::apply(paths, &bundle, plan.unwrap())
        .await
        .is_err();
    write_private_replace(&owner_path, &original).unwrap();
    assert!(stale_rejected);
    {
        // This process is deliberately NOT the manifest-bound CLI. It owns only this
        // private runtime and listener, and must never be admitted as that service.
        let guard = RuntimeGuard::acquire(&location, &paths.state).unwrap();
        let listener =
            std::os::unix::net::UnixListener::bind(location.path.join("service.sock")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            location.path.join("service.sock"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let (unknown, plan) = lifecycle::plan(paths, Some(&bundle), "start")
            .await
            .unwrap();
        assert_eq!(unknown["state"], "unknown");
        assert_eq!(unknown["canApply"], false);
        assert!(plan.is_none());
        assert!(unknown["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "unowned_service"));
        drop(listener);
        drop(guard);
    }
    json!({"ok":true,"case":case,"expiredPlanRefused":true,"staleGenerationRefused":true,"unknownExecutableRefused":true})
}
fn cli_matches(pid: i32, bundle: &Bundle) -> bool {
    let mut bytes = vec![0u8; 4096];
    let n = unsafe { libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
    if n <= 0 {
        return false;
    }
    bytes.truncate(n as usize);
    if bytes.last() == Some(&0) {
        bytes.pop();
    }
    use std::os::unix::ffi::OsStringExt;
    let path = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    bundle.cli().is_ok_and(|p| p == path)
}
fn finish(
    app: tauri::AppHandle<tauri::Wry>,
    paths: Paths,
    mut result: Value,
    seen: &Mutex<BTreeMap<i32, runtime::RuntimeOwner>>,
) {
    let control = app.state::<Mutex<held::Controller>>();
    let mut control = control.lock().unwrap();
    let cleanup = control.restore(&paths);
    let report = control.report();
    if report["used"] != 0 {
        result["nativeHeld"] = report;
        result["nativeHeldCleanup"] = match &cleanup {
            Ok(receipt) => json!({"ok":true,"receipt":receipt}),
            Err(error) => json!({"ok":false,"error":error}),
        };
    }
    if cleanup.is_err() || result.get("nativeHeld").is_some_and(|v| v["clean"] != true) {
        result["ok"] = json!(false);
    }
    drop(control);
    let location = paths.runtime_location(&std::env::vars().collect()).unwrap();
    let bundle = app.state::<Bundle>().inner().clone();
    let mut shutdown_ack = None;
    if let Ok(owner) = runtime::verify_active(&location.path, &paths.state) {
        if lifecycle::owned_service(&owner, &bundle).unwrap_or(false)
            && cli_matches(owner.pid, &bundle)
        {
            seen.lock().unwrap().insert(owner.pid, owner);
            shutdown_ack = Some(
                tauri::async_runtime::block_on(omm::service::daemon::request(
                    &paths, "shutdown", None,
                ))
                .is_ok(),
            );
        }
    }
    let owners: Vec<_> = seen.lock().unwrap().values().cloned().collect();
    for _ in 0..100 {
        if owners.iter().all(|o| !runtime::owner_is_live(o))
            && profile::offline_lock(&paths).is_ok()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    // Core owns its CLI reaper thread. Owner disappearance is not a fixture Child wait receipt.
    result["coreCliCleanup"] = json!({"shutdownAcknowledged":shutdown_ack,"owners":owners,"fixtureChildWaited":false,"coreWaitResultObserved":false,"reaperSource":"src/application/lifecycle.rs:start -> child.wait","forcedSignals":0});
    let all_exited = owners.iter().all(|o| !runtime::owner_is_live(o));
    let locks_released = profile::offline_lock(&paths).is_ok();
    let socket_removed = !location.path.join("service.sock").exists();
    let helper = app
        .state::<Mutex<Option<capture::ChildGuard>>>()
        .lock()
        .unwrap()
        .take();
    let helper_receipt = helper.map(|mut helper| {
        let pid = helper.0.id();
        let mut status = None;
        for _ in 0..100 {
            status = helper.0.try_wait().ok().flatten();
            if status.is_some() { break; }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        json!({"pid":pid,"exitedByServiceQuit":status.is_some(),"waited":status.is_some(),"exitCode":status.and_then(|s|s.code()),"forcedSignals":0})
    });
    result["capturePeer"] = json!(helper_receipt);
    let cases = app.state::<Mutex<Vec<Value>>>().lock().unwrap().clone();
    if cases.len() != 4 || cases.iter().any(|c| c["ok"] != true) {
        result["ok"] = json!(false);
    }
    result["nativeCases"] = json!(cases);
    result["nativeLifecycle"] = json!({"ownedPids":owners.iter().map(|o|o.pid).collect::<Vec<_>>(),"allOwnedExited":all_exited,"locksReleased":locks_released,"socketRemoved":socket_removed,"bundleSource":bundle.manifest.source_revision,"manifestHash":bundle.manifest_hash,"cliHash":bundle.manifest.files["bin/omm"],"dataPreserved":std::fs::read(paths.data.join("lifecycle-preserve.txt")).is_ok_and(|b|b==b"private synthetic data must survive start-stop-switch")});
    if !all_exited
        || !locks_released
        || !socket_removed
        || owners.len() != 2
        || result["nativeLifecycle"]["dataPreserved"] != true
    {
        result["ok"] = json!(false);
    }
    write_private_new(
        &output("bridge-result.json"),
        &serde_json::to_vec_pretty(
            &json!({"result":result,"profile":"new private lifecycle profile"}),
        )
        .unwrap(),
    )
    .unwrap();
    let safe_to_release = all_exited && locks_released && socket_removed && cleanup.is_ok();
    if safe_to_release {
        children::core_pending(false);
    }
    if safe_to_release && !children::exit_blocked() {
        app.exit(if result["ok"] == true { 0 } else { 1 });
    } else {
        write_private_new(&output("retained-control.json"), &serde_json::to_vec_pretty(&json!({"hostPid":std::process::id(),"coreOwners":owners,"retainedChildPids":children::retained_ids(),"workspaceRetained":true,"forcedSignals":0,"reason":"normal cleanup incomplete; host retains control; no signals sent"})).unwrap()).unwrap();
    }
}

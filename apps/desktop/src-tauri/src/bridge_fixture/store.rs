//! Explicit UUID-owned WK stores. Never opens or removes the default store.
use super::*;
use objc2::{
    msg_send,
    runtime::{AnyClass, AnyObject},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs::File, os::fd::AsRawFd, sync::mpsc, time::Duration};
use tauri::{WebviewUrl, WebviewWindow, WebviewWindowBuilder};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    uid: u32,
    ids: [Uuid; 2],
    nonce: Uuid,
    removed: bool,
}
struct Lease {
    manifest: Manifest,
    path: PathBuf,
    _lock: File,
}
fn manifest_valid(manifest: &Manifest) -> bool {
    manifest.schema_version == 1
        && manifest.uid == unsafe { libc::geteuid() }
        && !manifest.removed
        && manifest.ids[0] != manifest.ids[1]
        && manifest.ids.iter().all(|id| id.get_version_num() == 4)
        && manifest.nonce.get_version_num() == 4
}
fn supported() {
    let class = AnyClass::get(c"WKWebsiteDataStore").expect("WKWebsiteDataStore");
    let supported: bool =
        unsafe { msg_send![class, respondsToSelector: objc2::sel!(dataStoreForIdentifier:)] };
    assert!(
        supported,
        "custom persistent WK stores unavailable; default fallback forbidden"
    );
}
pub fn requested() -> bool {
    std::env::var_os("OMM_WKWEBVIEW_STORE_MANIFEST").is_some()
}
pub fn configure(context: &mut tauri::Context<tauri::Wry>) {
    if scenario() == "store" || requested() {
        supported();
        assert!(
            scenario() == "store" || scenario() == "ui",
            "owned store only allowed for explicit store/UI scenes"
        );
        context.config_mut().app.windows[0].create = false;
    }
}
fn load() -> Lease {
    let supplied = PathBuf::from(
        std::env::var_os("OMM_WKWEBVIEW_STORE_MANIFEST").expect("owned manifest required"),
    );
    load_path(supplied)
}
fn load_path(supplied: PathBuf) -> Lease {
    assert!(
        supplied.starts_with("/private/tmp/")
            && supplied.file_name().unwrap() == "owned-wk-store.json"
    );
    assert_eq!(supplied.canonicalize().unwrap(), supplied);
    validate_private_dir(supplied.parent().unwrap()).unwrap();
    validate_private_file(&supplied).unwrap();
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(&supplied).unwrap()).unwrap();
    assert!(
        manifest_valid(&manifest),
        "invalid or removed owned store manifest"
    );
    let path = supplied.parent().unwrap().join("store-use.lock");
    if !path.exists() {
        write_private_new(&path, b"").unwrap();
    }
    validate_private_file(&path).unwrap();
    let lock = File::open(path).unwrap();
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "owned WK store already in use"
    );
    Lease {
        manifest,
        path: supplied,
        _lock: lock,
    }
}
fn evaluate(view: &WebviewWindow, script: &str) -> Value {
    let (tx, rx) = mpsc::sync_channel(1);
    view.eval_with_callback(script, move |text| {
        let _ = tx.send(text);
    })
    .unwrap();
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(text) => serde_json::from_str(&text).unwrap(),
        Err(mpsc::RecvTimeoutError::Disconnected) => Value::Null,
        Err(error) => panic!("owned WK evaluation timed out: {error}"),
    }
}
fn decoded(view: &WebviewWindow, script: &str) -> Value {
    let value = evaluate(view, &format!("JSON.stringify({script})"));
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}
fn actual(view: &WebviewWindow, id: Uuid) {
    let (tx, rx) = mpsc::sync_channel(1);
    view.with_webview(move |view| unsafe {
        let config: *mut AnyObject = msg_send![view.inner().cast::<AnyObject>(), configuration];
        let store: *mut AnyObject = msg_send![config, websiteDataStore];
        let persistent: bool = msg_send![store, isPersistent];
        let identifier: *mut AnyObject = msg_send![store, identifier];
        let mut bytes = [0u8; 16];
        if !identifier.is_null() {
            let _: () = msg_send![identifier,getUUIDBytes:bytes.as_mut_ptr()];
        }
        let _ = tx.send((persistent, bytes));
    })
    .unwrap();
    let (persistent, bytes) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        persistent && bytes == *id.as_bytes(),
        "actual WK store differs; never navigate to app content"
    );
}
fn view(app: &tauri::AppHandle, label: &str, id: Uuid) -> WebviewWindow {
    let view = WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::External("about:blank".parse().unwrap()),
    )
    .incognito(false)
    .on_web_resource_request(|_, response| {
        *response.body_mut() =
            b"<!doctype html><meta charset=utf-8><title>Owned WK store proof</title>"
                .to_vec()
                .into();
        response
            .headers_mut()
            .insert("Content-Type", "text/html; charset=utf-8".parse().unwrap());
    })
    .data_store_identifier(*id.as_bytes())
    .visible(false)
    .build()
    .unwrap();
    actual(&view, id);
    view
}
fn ready(view: &WebviewWindow) {
    view.navigate("tauri://localhost/".parse().unwrap())
        .unwrap();
    for _ in 0..100 {
        if evaluate(view, "location.origin") == "tauri://localhost"
            && evaluate(view, "document.readyState") == "complete"
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("owned store local page not ready");
}
fn marker(view: &WebviewWindow) -> Value {
    decoded(view, "localStorage.getItem('__omm_owned_wk_nonce')")
}
fn set_marker(view: &WebviewWindow, nonce: Uuid) {
    evaluate(
        view,
        &format!(
            "localStorage.setItem('__omm_owned_wk_nonce',{nonce:?});true",
            nonce = nonce.to_string()
        ),
    );
}
fn destroy(app: &tauri::AppHandle, view: WebviewWindow) {
    let label = view.label().to_string();
    view.destroy().unwrap();
    drop(view);
    for _ in 0..100 {
        if app.get_webview_window(&label).is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("owned store view did not destroy");
}
fn ids(app: &tauri::AppHandle) -> Vec<[u8; 16]> {
    tauri::async_runtime::block_on(app.fetch_data_store_identifiers()).unwrap()
}
fn allocate(app: &tauri::AppHandle) -> Lease {
    assert!(
        !requested(),
        "allocate never accepts a caller-selected UUID"
    );
    let existing = ids(app);
    let manifest = Manifest {
        schema_version: 1,
        uid: unsafe { libc::geteuid() },
        ids: [Uuid::new_v4(), Uuid::new_v4()],
        nonce: Uuid::new_v4(),
        removed: false,
    };
    assert!(manifest
        .ids
        .iter()
        .all(|id| !existing.contains(id.as_bytes())));
    let path = output("owned-wk-store.json");
    write_private_new(&path, &serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let lease = load_path(path);
    let a = view(app, "owned-store-a", lease.manifest.ids[0]);
    let b = view(app, "owned-store-b", lease.manifest.ids[1]);
    ready(&a);
    ready(&b);
    assert_eq!(marker(&a), Value::Null);
    assert_eq!(marker(&b), Value::Null);
    set_marker(&a, lease.manifest.nonce);
    assert_eq!(marker(&b), Value::Null);
    set_marker(&b, lease.manifest.ids[1]);
    assert_eq!(marker(&a), json!(lease.manifest.nonce.to_string()));
    destroy(app, a);
    destroy(app, b);
    lease
}
fn cleanup(app: &tauri::AppHandle, lease: &mut Lease) {
    for (index, id) in lease.manifest.ids.iter().enumerate() {
        if !ids(app).contains(id.as_bytes()) {
            continue;
        }
        let view = view(app, "owned-store-cleanup", *id);
        ready(&view);
        let expected = if index == 0 {
            lease.manifest.nonce
        } else {
            lease.manifest.ids[1]
        };
        assert_eq!(
            marker(&view),
            json!(expected.to_string()),
            "owned marker mismatch; removal refused"
        );
        destroy(app, view);
        let mut removed = false;
        for _ in 0..40 {
            if tauri::async_runtime::block_on(app.remove_data_store(*id.as_bytes())).is_ok() {
                removed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            removed,
            "exact owned WK store remains in use; no broad cleanup"
        );
    }
    let remaining = ids(app);
    assert!(lease
        .manifest
        .ids
        .iter()
        .all(|id| !remaining.contains(id.as_bytes())));
    lease.manifest.removed = true;
    write_private_replace(
        &lease.path,
        &serde_json::to_vec_pretty(&lease.manifest).unwrap(),
    )
    .unwrap();
}
pub fn ui_setup(app: &tauri::AppHandle) {
    if !requested() {
        return;
    }
    let lease = load();
    let id = lease.manifest.ids[0];
    let nonce = lease.manifest.nonce;
    app.manage(lease);
    let app = app.clone();
    std::thread::spawn(move || {
        let keeper = WebviewWindowBuilder::new(
            &app,
            "owned-store-keeper",
            WebviewUrl::External("about:blank".parse().unwrap()),
        )
        .incognito(true)
        .visible(false)
        .build()
        .unwrap();
        let mut config = app.config().app.windows[0].clone();
        config.url = WebviewUrl::External("about:blank".parse().unwrap());
        config.incognito = false;
        let main = WebviewWindowBuilder::from_config(&app, &config)
            .unwrap()
            .data_store_identifier(*id.as_bytes())
            .build()
            .unwrap();
        actual(&main, id);
        ready(&main);
        assert_eq!(marker(&main), json!(nonce.to_string()));
        main.eval(format!(
            "window.__ommOwnedStoreAttached={}",
            json!({"actualPersistent":true,"actualIdentifier":id,"ownedMarkerVerified":true})
        ))
        .unwrap();
        write_private_new(&output("owned-store-attached.json"),&serde_json::to_vec_pretty(&json!({"actualPersistent":true,"actualIdentifier":id,"ownedMarkerVerified":true,"defaultStoreUsed":false})).unwrap()).unwrap();
        destroy(&app, keeper);
    });
}
pub fn builder(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.setup(|app|{
        WebviewWindowBuilder::new(app,"owned-store-keeper",WebviewUrl::External("about:blank".parse().unwrap())).incognito(true).visible(false).build()?;
        let app=app.handle().clone();std::thread::spawn(move||{
            let action=std::env::var("OMM_WKWEBVIEW_STORE_ACTION").unwrap_or_else(|_|"proof".into());
            let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||{
                let mut lease=if action=="allocate" || action=="proof"{allocate(&app)}else{load()};
                match action.as_str(){
                    "proof"|"cleanup"=>cleanup(&app,&mut lease),
                    "allocate"=>{},
                    "write"|"read"=>{
                        let a=view(&app,"owned-store-probe",lease.manifest.ids[0]);ready(&a);assert_eq!(marker(&a),json!(lease.manifest.nonce.to_string()));
                        if action=="write" {evaluate(&a,"localStorage.setItem('__omm_cross_process_probe','private synthetic restart');true");}
                        else {assert_eq!(decoded(&a,"localStorage.getItem('__omm_cross_process_probe')"),"private synthetic restart");}
                        destroy(&app,a);
                    },
                    _=>panic!("unknown owned store action"),
                }
                json!({"ok":true,"action":action,"playCalls":0,"ids":lease.manifest.ids,"removed":lease.manifest.removed,"actualPersistent":true,"actualIdentifiersVerified":true,"independentStores":true,"defaultStoreUsed":false,"manifest":lease.path})
            })).unwrap_or_else(|_|json!({"ok":false,"action":action,"playCalls":0,"error":"owned WK store proof failed; no default fallback"}));
            write_private_new(&output("bridge-result.json"),&serde_json::to_vec_pretty(&json!({"result":result})).unwrap()).unwrap();
            app.exit(if result["ok"]==true{0}else{1});
        });Ok(())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_rejects_removed_duplicate_foreign_and_unknown_stores() {
        let mut manifest = Manifest {
            schema_version: 1,
            uid: unsafe { libc::geteuid() },
            ids: [Uuid::new_v4(), Uuid::new_v4()],
            nonce: Uuid::new_v4(),
            removed: false,
        };
        assert!(manifest_valid(&manifest));
        manifest.removed = true;
        assert!(!manifest_valid(&manifest));
        manifest.removed = false;
        manifest.uid = manifest.uid.wrapping_add(1);
        assert!(!manifest_valid(&manifest));
        manifest.uid = unsafe { libc::geteuid() };
        manifest.ids[1] = manifest.ids[0];
        assert!(!manifest_valid(&manifest));
        manifest.ids[1] = Uuid::new_v4();
        manifest.ids[0] = Uuid::nil();
        assert!(!manifest_valid(&manifest));
        manifest.ids[0] = Uuid::new_v4();
        let mut json = serde_json::to_value(&manifest).unwrap();
        json["arbitraryPath"] = json!("/private/tmp/not-owned");
        assert!(serde_json::from_value::<Manifest>(json).is_err());
        manifest.schema_version = 2;
        assert!(!manifest_valid(&manifest));
    }
}

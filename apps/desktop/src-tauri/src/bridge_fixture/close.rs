//! Feature-only native close requests and evidence; React alone decides whether to destroy main.
use super::*;
use serde_json::{json, Value};
use std::sync::Mutex;

const REQUEST: &str = "JSON.stringify((()=>{const r=window.__ommFixtureCloseRequest;window.__ommFixtureCloseRequest=null;return r?{request:r,dom:document.body.innerText.slice(0,65536),origin:location.origin}:null})())";
const KEEPER: &str = "fixture-close-keeper";

pub struct Controller {
    state: Mutex<State>,
    destroyed: AtomicBool,
}
#[derive(Default)]
struct State {
    ids: Vec<String>,
    armed: Option<Value>,
    completed: bool,
}
impl Controller {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
            destroyed: AtomicBool::new(false),
        }
    }
    pub fn destroyed(&self) {
        self.destroyed.store(true, Ordering::SeqCst);
    }
    pub fn armed(&self) -> bool {
        self.state.lock().unwrap().armed.is_some()
    }
    pub fn used(&self) -> bool {
        !self.state.lock().unwrap().ids.is_empty()
    }
    pub fn poll(self: &Arc<Self>, view: &tauri::Webview<tauri::Wry>) {
        let controller = self.clone();
        let view = view.clone();
        let owned = view.clone();
        let _ = view.eval_with_callback(REQUEST, move |text| {
            let Ok(Value::String(text)) = serde_json::from_str::<Value>(&text) else {
                return;
            };
            let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
                return;
            };
            if envelope.is_null() {
                return;
            }
            // WK evaluation callbacks run on the event thread. Window creation/close must
            // dispatch from an owned worker, not synchronously block that same event thread.
            let controller = controller.clone();
            let owned = owned.clone();
            std::thread::spawn(move || {
                let reply = controller.receive(&owned, &envelope);
                if trusted(&owned) {
                    let _ = owned.eval(format!("window.__ommFixtureCloseResult={reply}"));
                }
            });
        });
    }
    fn receive(&self, view: &tauri::Webview<tauri::Wry>, envelope: &Value) -> Value {
        let request = &envelope["request"];
        let reject = || json!({"ok":false,"error":"invalid owned-main close request"});
        if !trusted(view) || validate(view.label(), &view.url().unwrap(), request).is_err() {
            return reject();
        }
        let mut state = self.state.lock().unwrap();
        let id = request["id"].as_str().unwrap();
        if state.ids.len() >= 16 || state.ids.iter().any(|v| v == id) || state.armed.is_some() {
            return reject();
        }
        let app = view.app_handle();
        // Keep the fixture alive after main's real destruction, solely to drain owned resources.
        if app.get_webview_window(KEEPER).is_none()
            && tauri::WebviewWindowBuilder::new(
                app,
                KEEPER,
                tauri::WebviewUrl::App("index.html".into()),
            )
            .visible(false)
            .incognito(true)
            .on_web_resource_request(|_, response| {
                *response.body_mut() = std::borrow::Cow::Borrowed(
                    b"<!doctype html><title>Private close cleanup</title>",
                );
            })
            .build()
            .is_err()
        {
            return reject();
        }
        state.ids.push(id.into());
        if request["action"] == "arm-destroy" {
            let evidence = json!({"requestId":id,"result":request["result"],"dom":envelope["dom"],"origin":envelope["origin"],"mainAlive":true,"closeRequestIds":state.ids});
            if write_private_new(
                &output("close-before-destroy.json"),
                &serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .is_err()
            {
                return reject();
            }
            state.armed = Some(request["result"].clone());
            return json!({"ok":true,"id":id,"armed":true,"mainAlive":true});
        }
        match view.window().close() {
            Ok(()) => json!({"ok":true,"id":id,"nativeCloseRequested":true}),
            Err(_) => reject(),
        }
    }
    pub fn completion(&self, app: &tauri::AppHandle<tauri::Wry>) -> Option<Value> {
        let mut state = self.state.lock().unwrap();
        if state.completed || app.get_webview_window("main").is_some() {
            return None;
        }
        state.completed = true;
        let destroyed = self.destroyed.load(Ordering::SeqCst);
        let mut result = state.armed.clone().unwrap_or_else(|| {
            json!({"ok":false,"playCalls":0,"error":"main closed before close evidence was armed"})
        });
        result["nativeClose"] = json!({"destroyedEvent":destroyed,"mainAbsent":true,"closeRequestIds":state.ids,"beforeDestroyFile":"close-before-destroy.json"});
        if !destroyed {
            result["ok"] = json!(false);
        }
        Some(result)
    }
}
fn trusted(view: &tauri::Webview<tauri::Wry>) -> bool {
    view.label() == "main"
        && view
            .url()
            .is_ok_and(|url| url.scheme() == "tauri" && url.host_str() == Some("localhost"))
}
fn validate(label: &str, url: &tauri::Url, request: &Value) -> std::result::Result<(), ()> {
    if label != "main" || url.scheme() != "tauri" || url.host_str() != Some("localhost") {
        return Err(());
    }
    let object = request.as_object().ok_or(())?;
    let id = request["id"].as_str().ok_or(())?;
    if id.is_empty()
        || id.len() > 64
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || serde_json::to_vec(request).map_err(|_| ())?.len() > 65536
    {
        return Err(());
    }
    match request["action"].as_str() {
        Some("close") if object.len() == 2 => Ok(()),
        Some("arm-destroy") if object.len() == 3 && request["result"].is_object() => {
            let result = &request["result"];
            if result["ok"] != true
                || result["playCalls"] != 0
                || !result["checks"].as_array().is_some_and(|checks| {
                    !checks.is_empty()
                        && checks.len() <= 512
                        && checks
                            .iter()
                            .all(|c| c.as_str().is_some_and(|s| s.len() <= 512))
                })
            {
                return Err(());
            }
            Ok(())
        }
        _ => Err(()),
    }
}

pub fn finish(app: tauri::AppHandle<tauri::Wry>, paths: Paths, mut result: Value) {
    tauri::async_runtime::spawn(async move {
        // The real service seals capture and drains workers before acknowledging shutdown.
        let shutdown = omm::service::daemon::request(&paths, "shutdown", None)
            .await
            .is_ok();
        if !wait_fixture_service(&app, &paths) {
            result["ok"] = json!(false);
            result["error"] =
                json!("owned fixture CLI did not exit normally; control handle retained");
            write_private_new(
                &output("bridge-result.json"),
                &serde_json::to_vec_pretty(
                    &json!({"result":result,"profile":"private UI close fixture"}),
                )
                .unwrap(),
            )
            .unwrap();
            return;
        }
        let mut capture_waited = false;
        let mut capture_success = false;
        let helper = app
            .state::<Mutex<Option<capture::ChildGuard>>>()
            .lock()
            .unwrap()
            .take();
        if let Some(mut helper) = helper {
            for _ in 0..100 {
                if let Ok(Some(status)) = helper.0.try_wait() {
                    capture_success = status.success();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            // On a failed shutdown, ChildGuard kills/waits only this explicitly owned child.
            drop(helper);
            capture_waited = true;
        }
        let readers_closed = Store::open_read_only(&paths)
            .and_then(|s| s.active_audio_readers())
            .is_ok_and(|active| !active);
        result["nativeClose"]["shutdownAcknowledged"] = json!(shutdown);
        result["nativeClose"]["captureChildWaited"] = json!(capture_waited);
        result["nativeClose"]["captureExitSuccess"] = json!(capture_success);
        result["nativeClose"]["readersReleased"] = json!(readers_closed);
        if !shutdown || !capture_waited || !capture_success || !readers_closed {
            result["ok"] = json!(false);
        }
        write_private_new(
            &output("bridge-result.json"),
            &serde_json::to_vec_pretty(
                &json!({"result":result,"profile":"private UI close fixture"}),
            )
            .unwrap(),
        )
        .unwrap();
        app.exit(if result["ok"] == true { 0 } else { 1 });
    });
}

pub fn assert_untrusted(view: &tauri::Webview<tauri::Wry>) {
    let controller = Controller::new();
    let result = controller.receive(view, &json!({"request":{"id":"denied-1","action":"close"}}));
    assert_eq!(result["ok"], false);
    assert!(view.app_handle().get_webview_window(view.label()).is_some());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_close_accepts_only_bounded_local_main_requests() {
        let local = "tauri://localhost/index.html".parse().unwrap();
        let close = json!({"id":"close-1","action":"close"});
        let arm = json!({"id":"arm-1","action":"arm-destroy","result":{"ok":true,"playCalls":0,"checks":["React confirmation ready"]}});
        assert!(validate("main", &local, &close).is_ok());
        assert!(validate("main", &local, &arm).is_ok());
        assert!(validate("unknown", &local, &close).is_err());
        assert!(validate("main", &"http://127.0.0.1:1420/".parse().unwrap(), &close).is_err());
        for invalid in [
            json!({"id":"../escape","action":"close"}),
            json!({"id":"x","action":"destroy"}),
            json!({"id":"x","action":"close","label":"other"}),
            json!({"id":"x","action":"arm-destroy","result":{"ok":true,"checks":[],"playCalls":0}}),
            json!({"id":"x","action":"arm-destroy","result":{"ok":true,"checks":["x"],"playCalls":1}}),
        ] {
            assert!(validate("main", &local, &invalid).is_err());
        }
    }
}

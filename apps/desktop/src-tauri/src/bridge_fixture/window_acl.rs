//! WK checks use the unchanged production capability; no test IPC permission is added.
use super::*;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::{sync::mpsc, time::Duration};
use tauri::{Emitter, WebviewUrl, WebviewWindowBuilder};
fn remote_page() -> (tauri::Url, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let thread = std::thread::spawn(move || {
        for _ in 0..100 {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut request = [0; 2048];
                    let Ok(count) = stream.read(&mut request) else {
                        continue;
                    };
                    if !request[..count].starts_with(b"GET / HTTP/") {
                        continue;
                    }
                    let body = "<!doctype html><title>Private remote ACL fixture</title>";
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(e) => panic!("private HTTP fixture: {e}"),
            }
        }
        panic!("private HTTP page was not requested");
    });
    (url, thread)
}

fn evaluate(view: &tauri::Webview<tauri::Wry>, script: &str) -> Value {
    let (send, receive) = mpsc::sync_channel(1);
    view.eval_with_callback(script, move |value| {
        let _ = send.send(value);
    })
    .unwrap();
    match receive.recv_timeout(Duration::from_secs(5)) {
        Ok(value) => serde_json::from_str(&value).unwrap(),
        Err(mpsc::RecvTimeoutError::Disconnected) => Value::Null, // A new WK view may not yet have its JS context.
        Err(error) => panic!("WK evaluation callback: {error}"),
    }
}
fn probe(view: &tauri::Webview<tauri::Wry>, script: &str) -> Value {
    evaluate(view, &format!("window.__ommAclResult=null; (async()=>{{try{{{script}}}catch(e){{window.__ommAclResult={{ok:false,error:String(e)}}}}}})(); 'scheduled'"));
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(100));
        let text = evaluate(view, "JSON.stringify(window.__ommAclResult||null)");
        let result: Value = serde_json::from_str(text.as_str().unwrap()).unwrap();
        if result.is_object() {
            assert_eq!(result["ok"], true, "{result}");
            return result;
        }
    }
    panic!("WK ACL probe timed out")
}
fn ready(view: &tauri::Webview<tauri::Wry>) {
    ready_at(view, "tauri://localhost");
}
fn ready_at(view: &tauri::Webview<tauri::Wry>, origin: &str) {
    for _ in 0..50 {
        if evaluate(
            view,
            &format!("!!window.__TAURI_INTERNALS__ && location.origin=={origin:?}"),
        ) == true
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("WK IPC injection not ready: {}", evaluate(view, "JSON.stringify({url:location.href,ipc:typeof window.__TAURI_INTERNALS__,body:document.body?.innerText})"))
}
const NEGATIVE: &str = r#"
 const checks=[]; const invoke=window.__TAURI_INTERNALS__.invoke;
 for(const [command,args] of [
 ['plugin:event|listen',{event:'tauri://close-requested',target:{kind:'Window',label:'main'},handler:window.__TAURI_INTERNALS__.transformCallback(()=>{})}],
 ['plugin:event|unlisten',{event:'tauri://close-requested',eventId:1}],
 ['plugin:window|destroy',{label:'main'}],
 ['plugin:event|emit',{event:'omm-fixture-denied',payload:null}]
 ]) {let denied=false;try{await invoke(command,args)}catch(e){denied=/not allowed|denied|not permitted/i.test(String(e));checks.push({command,error:String(e)})}if(!denied)throw Error('expected ACL denial: '+command)}
 window.__ommAclResult={ok:true,checks,origin:location.origin};
"#;
pub fn start(view: tauri::Webview<tauri::Wry>, paths: Paths) {
    std::thread::spawn(move || {
        let app = view.app_handle().clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            eprintln!("ACL stage: local main ready");
            ready(&view);
            let storage_key = format!("omm-fixture-storage-{}", Uuid::new_v4());
            assert_eq!(
                evaluate(
                    &view,
                    &format!(
                        "localStorage.setItem({0:?}, 'A'); localStorage.getItem({0:?})",
                        storage_key
                    )
                ),
                "A"
            );
            let allowed = probe(
                &view,
                r#"
 const invoke=window.__TAURI_INTERNALS__.invoke;window.__ommAclCloseCount=0;
 const handler=window.__TAURI_INTERNALS__.transformCallback(()=>{window.__ommAclCloseCount++});
 window.__ommAclListener=await invoke('plugin:event|listen',{event:'tauri://close-requested',target:{kind:'Window',label:'main'},handler});
 let denied=false;try{await invoke('plugin:event|emit',{event:'omm-fixture-denied',payload:null})}catch(e){denied=/not allowed|denied|not permitted/i.test(String(e))}if(!denied)throw Error('emit unexpectedly allowed');
 window.__ommAclResult={ok:true,listen:true,unrelatedEmitDenied:true,origin:location.origin};
 "#,
            );
            assert_eq!(allowed["origin"], "tauri://localhost");
            eprintln!("ACL stage: pending close");
            app.get_webview_window("main").unwrap().close().unwrap();
            for _ in 0..40 {
                if evaluate(&view, "window.__ommAclCloseCount") == 1 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(evaluate(&view, "window.__ommAclCloseCount"), 1);
            assert!(
                app.get_webview_window("main").is_some(),
                "listener must preserve pending close"
            );
            probe(
                &view,
                r#"
 await window.__TAURI_INTERNALS__.invoke('plugin:event|unlisten',{event:'tauri://close-requested',eventId:window.__ommAclListener});
 window.__ommAclResult={ok:true,unlisten:true};
 "#,
            );
            app.emit_to("main", "tauri://close-requested", ()).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(
                evaluate(&view, "window.__ommAclCloseCount"),
                1,
                "unlisten must remove callback"
            );
            // A separate unprivileged local window keeps the application alive for the destroy check.
            eprintln!("ACL stage: unknown local");
            let unknown = WebviewWindowBuilder::new(
                &app,
                "acl-unknown",
                WebviewUrl::App("index.html".into()),
            )
            .visible(false)
            .incognito(true)
            .build()
            .unwrap();
            let unknown_view: tauri::Webview<tauri::Wry> = unknown.as_ref().clone();
            ready(&unknown_view);
            assert_eq!(
                evaluate(
                    &unknown_view,
                    &format!("localStorage.getItem({storage_key:?})")
                ),
                Value::Null,
                "independent incognito store B must not see A"
            );
            assert_eq!(
                evaluate(
                    &unknown_view,
                    &format!(
                        "localStorage.setItem({0:?}, 'B'); localStorage.getItem({0:?})",
                        storage_key
                    )
                ),
                "B"
            );
            assert_eq!(
                evaluate(&view, &format!("localStorage.getItem({storage_key:?})")),
                "A",
                "store A must not see B's update"
            );
            let unknown_result = probe(&unknown_view, NEGATIVE);
            super::close::assert_untrusted(&unknown_view);
            // Navigate the same owned main window: label remains trusted, origin must not.
            // Destroy/recreate of that label races asynchronous native WK teardown and is not required.
            eprintln!("ACL stage: remote main navigation");
            let (remote_url, server) = remote_page();
            let expected_origin = remote_url.origin().ascii_serialization();
            app.get_webview_window("main")
                .unwrap()
                .navigate(remote_url)
                .unwrap();
            ready_at(&view, &expected_origin);
            let remote_result = probe(&view, NEGATIVE);
            super::close::assert_untrusted(&view);
            assert_eq!(remote_result["origin"], expected_origin);
            server.join().unwrap();
            app.get_webview_window("main")
                .unwrap()
                .navigate("tauri://localhost/index.html".parse().unwrap())
                .unwrap();
            ready(&view);
            eprintln!("ACL stage: actual main destroy");
            view.eval("window.__TAURI_INTERNALS__.invoke('plugin:window|destroy',{label:'main'}).catch(e=>console.error(e))").unwrap();
            for _ in 0..40 {
                if app.get_webview_window("main").is_none() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(
                app.get_webview_window("main").is_none(),
                "permitted main destroy must actually close its window"
            );
            json!({"ok":true,"playCalls":0,"allowed":allowed,"storageIsolation":{"incognito":true,"sameOriginIndependentWindows":true,"aSeesA":true,"bInitiallyEmpty":true,"bSeesB":true,"crossProcessPersistenceTested":false},"pendingClosePreserved":true,"unlistenRemovedCallback":true,"mainActuallyDestroyed":true,"unknownLocalWindow":unknown_result,"remoteMainWindow":remote_result,"fixtureCloseUnknownDenied":true,"fixtureCloseRemoteDenied":true})
        }));
        let result = result.unwrap_or_else(|_| json!({"ok":false,"error":"native WK ACL assertion failed; inspect runtime.log","playCalls":0}));
        write_private_new(
            &output("bridge-result.json"),
            &serde_json::to_vec_pretty(
                &json!({"result":result,"profile":"private window ACL fixture"}),
            )
            .unwrap(),
        )
        .unwrap();
        tauri::async_runtime::block_on(async {
            let _ = omm::service::daemon::request(&paths, "shutdown", None).await;
        });
        if super::wait_fixture_service(&app, &paths) && !super::children::exit_blocked() {
            app.exit(if result["ok"] == true { 0 } else { 1 });
        }
    });
}

//! Feature-only snapshots of the current owned WK view, never of a display or other window.
use super::*;
use objc2::{
    msg_send,
    runtime::{AnyClass, AnyObject},
};
use serde_json::{json, Value};

const REQUEST: &str = "JSON.stringify((()=>{const r=window.__ommFixtureSnapshotRequest;window.__ommFixtureSnapshotRequest=null;return r?{request:r,viewport:[innerWidth,innerHeight]}:null})())";

fn finish(view: &tauri::Webview<tauri::Wry>, result: Value) {
    view.eval(format!("window.__ommFixtureSnapshotResult={result}"))
        .unwrap();
}
pub fn poll(view: &tauri::Webview<tauri::Wry>) {
    let owned = view.clone();
    view.eval_with_callback(REQUEST, move |text| receive(&owned, &text))
        .unwrap();
}
fn receive(view: &tauri::Webview<tauri::Wry>, text: &str) {
    let Ok(Value::String(text)) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    if envelope.is_null() {
        return;
    }
    let request = &envelope["request"];
    let name = request["name"].as_str().unwrap_or("");
    let valid_name = !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    let valid_fields = request.as_object().is_some_and(|m| {
        m.keys()
            .all(|k| matches!(k.as_str(), "name" | "width" | "height"))
    });
    let local_main = view.label() == "main"
        && view
            .url()
            .is_ok_and(|u| u.scheme() == "tauri" && u.host_str() == Some("localhost"));
    if !valid_name || !valid_fields || !local_main {
        finish(
            view,
            json!({"ok":false,"error":"invalid owned-view snapshot request"}),
        );
        return;
    }
    let dimensions = if request["width"].is_null() && request["height"].is_null() {
        None
    } else {
        let (Some(w), Some(h)) = (request["width"].as_u64(), request["height"].as_u64()) else {
            finish(
                view,
                json!({"ok":false,"error":"snapshot dimensions required"}),
            );
            return;
        };
        if !(360..=1600).contains(&w) || !(480..=1200).contains(&h) {
            finish(
                view,
                json!({"ok":false,"error":"snapshot dimensions out of bounds"}),
            );
            return;
        }
        let before = view
            .window()
            .inner_size()
            .unwrap()
            .to_logical::<u32>(view.window().scale_factor().unwrap());
        // Preserve the measured WK titlebar inset instead of guessing a CSS viewport height.
        let viewport = &envelope["viewport"];
        let expected = json!([
            viewport[0].as_i64().unwrap() + w as i64 - before.width as i64,
            viewport[1].as_i64().unwrap() + h as i64 - before.height as i64
        ]);
        // Only this fixture-owned window. No IPC window-management grant.
        view.window()
            .set_min_size(Some(tauri::LogicalSize::new(360, 480)))
            .unwrap();
        view.window()
            .set_size(tauri::LogicalSize::new(w as u32, h as u32))
            .unwrap();
        Some((w, h, expected))
    };
    let view = view.clone();
    let name = name.to_owned();
    std::thread::spawn(move || {
        if let Some((w, h, expected)) = dimensions {
            if !wait_for_resize(&view, w, h, &expected) {
                finish(
                    &view,
                    json!({"ok":false,"name":name,"error":"owned viewport resize did not settle","requested":[w,h],"expectedViewport":expected}),
                );
                return;
            }
        }
        take_snapshot(&view, name);
    });
}
fn wait_for_resize(view: &tauri::Webview<tauri::Wry>, w: u64, h: u64, expected: &Value) -> bool {
    for _ in 0..40 {
        let (send, receive) = std::sync::mpsc::channel();
        view.eval_with_callback("JSON.stringify([innerWidth,innerHeight])", move |text| {
            let _ = send.send(text);
        })
        .unwrap();
        if let Ok(text) = receive.recv_timeout(std::time::Duration::from_millis(500)) {
            let observed = serde_json::from_str::<Value>(&text).ok().and_then(|v| {
                v.as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
            });
            let scale = view.window().scale_factor().unwrap();
            let window = view.window().inner_size().unwrap().to_logical::<u32>(scale);
            let bounds = view.bounds().unwrap().size.to_logical::<u32>(scale);
            if window.width as u64 == w
                && window.height as u64 == h
                && bounds.width as u64 == w
                && bounds.height as u64 == h
                && observed.as_ref() == Some(expected)
            {
                return true;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    false
}
fn take_snapshot(view: &tauri::Webview<tauri::Wry>, name: String) {
    let file = output(&format!("{name}.png"));
    let done = view.clone();
    view.with_webview(move |native| {
        let callback = block2::RcBlock::new(move |image:*mut AnyObject,error:*mut AnyObject| {
            let result = if !error.is_null() || image.is_null() { Err("WK takeSnapshot failed".into()) }
                else {
                    // The callback owns the NSImage lifetime; conversion objects stay valid until return.
                    unsafe { png(image) }.and_then(|bytes| {
                        write_private_new(&file,&bytes).map_err(|e|e.to_string())?;
                        Ok(bytes.len())
                    })
                };
            finish(&done,match result {
                Ok(bytes) => json!({"ok":true,"name":name,"bytes":bytes,"kind":"owned WKWebView content only"}),
                Err(error) => json!({"ok":false,"name":name,"error":error})
            });
        });
        // Main-thread WK API. No display capture, other windows, arbitrary paths or permissions.
        unsafe {
            let wk = native.inner().cast::<AnyObject>();
            let config:*mut AnyObject = msg_send![AnyClass::get(c"WKSnapshotConfiguration").unwrap(), new];
            let _:() = msg_send![config, setAfterScreenUpdates:true];
            let _:() = msg_send![wk, takeSnapshotWithConfiguration:config, completionHandler:&*callback];
            let _:() = msg_send![config, release];
        }
    }).unwrap();
}
// Called only inside WK's main-thread snapshot callback, before autoreleased objects expire.
unsafe fn png(image: *mut AnyObject) -> std::result::Result<Vec<u8>, String> {
    let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
    if tiff.is_null() {
        return Err("WK snapshot has no TIFF representation".into());
    }
    let class = AnyClass::get(c"NSBitmapImageRep").ok_or("NSBitmapImageRep unavailable")?;
    let rep: *mut AnyObject = msg_send![class, imageRepWithData:tiff];
    if rep.is_null() {
        return Err("WK snapshot bitmap conversion failed".into());
    }
    let dictionary: *mut AnyObject = msg_send![AnyClass::get(c"NSDictionary").unwrap(), dictionary];
    let png: *mut AnyObject = msg_send![rep, representationUsingType:4usize, properties:dictionary];
    if png.is_null() {
        return Err("WK snapshot PNG conversion failed".into());
    }
    let length: usize = msg_send![png, length];
    let bytes: *const u8 = msg_send![png, bytes];
    if length == 0 || length > 32 * 1024 * 1024 || bytes.is_null() {
        return Err("WK snapshot PNG size invalid".into());
    }
    Ok(std::slice::from_raw_parts(bytes, length).to_vec())
}

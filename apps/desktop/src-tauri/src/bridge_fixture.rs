//! Private real-bridge driver. Entire module and its native evaluation channel are absent from production.
use omm::{
    application::media,
    core::{
        config::AppConfig,
        contracts::*,
        paths::*,
        store::{hash_bytes, Store},
    },
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::Manager;
use uuid::Uuid;
pub mod capture;
mod window_acl;
pub fn scenario() -> String {
    let scenario = std::env::var("OMM_WKWEBVIEW_SCENARIO").unwrap_or_else(|_| "native".into());
    assert!(
        matches!(scenario.as_str(), "native" | "window-acl"),
        "unknown private fixture scenario"
    );
    scenario
}
pub fn output(name: &str) -> PathBuf {
    assert!(!name.contains('/') && !name.contains('\\') && name != "." && name != "..");
    let root = PathBuf::from(std::env::var_os("OMM_NATIVE_FIXTURE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    assert!(root.starts_with("/private/tmp/"));
    validate_private_dir(&root).unwrap();
    root.join(name)
}
pub fn prepare() -> Paths {
    let root =
        PathBuf::from(std::env::var_os("OMM_NATIVE_FIXTURE_ROOT").expect("private fixture root"))
            .canonicalize()
            .expect("fixture root");
    assert!(root.starts_with("/private/tmp/"));
    validate_private_dir(&root).unwrap();
    validate_private_file(&root.join("driver.js")).unwrap();
    let paths = Paths::from_env().unwrap();
    for path in [&paths.data, &paths.state, &paths.config, &paths.cache] {
        assert!(path.starts_with(&root));
    }
    assert!(
        !paths.state.join("state.sqlite").exists(),
        "fixture profile must be new"
    );
    paths.create_private_dirs().unwrap();
    write_private_new(&output("import.wav"), &media::silent_wav(1)).unwrap();
    let codec = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/audio-toolchain");
    if codec.join("manifest.json").is_file() {
        omm::core::audio_toolchain::install(&paths, &codec.canonicalize().unwrap()).unwrap();
    }
    let mut cfg = AppConfig::default();
    cfg.notifications.enabled = false;
    use std::os::unix::fs::PermissionsExt;
    let fake_pi = output("fake-pi");
    write_private_new(&fake_pi,b"#!/bin/sh\n/bin/cat >/dev/null\nprintf '# Private synthetic minutes\\n- [ ] Read-only fixture action\\n'\n").unwrap();
    std::fs::set_permissions(&fake_pi, std::fs::Permissions::from_mode(0o700)).unwrap();
    cfg.pi.executable = fake_pi;
    let fake_worker = output("fake-worker");
    write_private_new(&fake_worker, include_bytes!("../fixtures/fake-worker.sh")).unwrap();
    std::fs::set_permissions(&fake_worker, std::fs::Permissions::from_mode(0o700)).unwrap();
    cfg.worker_executable = fake_worker;
    cfg.asr.model_version = "fixture-asr".into();
    cfg.pi.provider = "fixture".into();
    cfg.pi.model = "fixture".into();
    cfg.save(&paths).unwrap();
    let id = Uuid::parse_str("00000000-0000-4000-8000-000000000041").unwrap();
    let directory = paths.data.join("meetings").join(id.to_string());
    private_dir(&directory).unwrap();
    let wav = media::silent_wav(3);
    let path = directory.join("segment.wav");
    write_private_new(&path, &wav).unwrap();
    let meeting = SealedMeeting {
        session_id: id,
        title: "Private bridge fixture".into(),
        directory,
        sealed_at_ms: chrono::Utc::now().timestamp_millis(),
        segments: vec![AudioSegment {
            capture: None,
            path,
            duration_ms: 3000,
            sha256: hash_bytes(&wav),
        }],
    };
    let mut store = Store::open(&paths).unwrap();
    store
        .set_state("capture_instance", &Uuid::new_v4())
        .unwrap();
    store.enqueue(&meeting, &cfg.snapshot(), false).unwrap();
    let claim = store.claim_asr(1, 10000).unwrap().unwrap();
    store
        .commit_transcript(
            &claim,
            &Transcript {
                schema_version: 1,
                model_version: claim.context.config.asr.model_version.clone(),
                duration_ms: 3000,
                segments: vec![TranscriptSegment {
                    source: AudioSource::Mic,
                    start_ms: 0,
                    end_ms: 3000,
                    text: "Private synthetic transcript token".into(),
                }],
            },
            2,
        )
        .unwrap();
    let claim = store.claim_summary(3, 10000).unwrap().unwrap();
    store
        .commit_summary(
            &claim,
            &Summary {
                schema_version: 1,
                markdown: "# Synthetic summary\n\n- [ ] Read-only synthetic action".into(),
                provider: "fixture".into(),
                model: "fixture".into(),
                prompt_version: claim.context.config.pi.prompt_version.clone(),
            },
            4,
        )
        .unwrap();
    paths
}
pub fn builder(builder: tauri::Builder<tauri::Wry>, paths: &Paths) -> tauri::Builder<tauri::Wry> {
    let root = PathBuf::from(std::env::var_os("OMM_NATIVE_FIXTURE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    let driver = std::fs::read_to_string(root.join("driver.js")).unwrap();
    assert!(driver.len() < 128 * 1024);
    let owned = paths.clone();
    let stop = paths.clone();
    let started = Arc::new(AtomicBool::new(false));
    let helper = capture::start(paths);
    builder
        .manage(std::sync::Mutex::new(helper))
        .setup(move |app| {
            if scenario() == "window-acl" {
                // Same local origin and capability, but no React listener competes with this API-level probe.
                tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                    .on_web_resource_request(|_, response| { *response.body_mut() = std::borrow::Cow::Borrowed(b"<!doctype html><title>Private window ACL fixture</title>"); })
                    .build()?;
            }
            let location = owned.runtime_location(&std::env::vars().collect()).unwrap();
            tauri::async_runtime::spawn(async move {
                omm::service::daemon::run_at(owned, location)
                    .await
                    .expect("private service fixture");
            });
            Ok(())
        })
        .on_page_load(move |view, event| {
            if event.event() != tauri::webview::PageLoadEvent::Finished
                || view.label() != "main"
                || started.swap(true, Ordering::SeqCst)
            {
                return;
            }
            if scenario() == "window-acl" {
                window_acl::start(view.clone(), stop.clone());
                return;
            }
            view.eval_with_callback(
                format!("(function(){{try {{{driver};return 'scheduled'}}catch(e){{return String(e)}}}})()"),
                |result| eprintln!("private driver scheduled: {result}"),
            ).unwrap();
            let view = view.clone();
            let root = root.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let done = Arc::new(AtomicBool::new(false));
                for _ in 0..70 {
                    if done.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    let root = root.clone();
                    let paths = stop.clone();
                    let done = done.clone();
                    let app = view.app_handle().clone();
                    view.eval_with_callback("JSON.stringify(window.__ommFixtureResult||null)", move |text| {
                        let Ok(serde_json::Value::String(text)) = serde_json::from_str(&text) else { return; };
                        let Ok(result) = serde_json::from_str::<serde_json::Value>(&text) else { return; };
                        if !result.is_object() || done.swap(true, Ordering::SeqCst) { return; }
                        write_private_new(
                            &root.join("bridge-result.json"),
                            &serde_json::to_vec_pretty(&serde_json::json!({"result":result,"profile":"private synthetic fixture"})).unwrap(),
                        ).unwrap();
                        let paths = paths.clone();
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = omm::service::daemon::request(&paths, "shutdown", None).await;
                            app.exit(if result["ok"] == true { 0 } else { 1 });
                        });
                    }).unwrap();
                }
            });
        })
}

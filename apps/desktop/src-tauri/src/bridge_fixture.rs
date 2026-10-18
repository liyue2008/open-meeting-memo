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
mod close;
mod lifecycle;
pub mod recovery;
mod review;
mod snapshot;
pub mod store;
mod window_acl;
pub fn review_cleanup() {
    review::cleanup_without_window();
}
pub fn run_lifecycle_unknown_listener() {
    lifecycle::run_unknown_listener();
}
pub fn scenario() -> String {
    let scenario = std::env::var("OMM_WKWEBVIEW_SCENARIO").unwrap_or_else(|_| "native".into());
    assert!(
        matches!(
            scenario.as_str(),
            "native" | "window-acl" | "ui" | "lifecycle" | "recovery" | "store"
        ),
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
fn archive_requested() -> bool {
    let Some(flag) = std::env::var_os("OMM_NATIVE_ARCHIVE_MEDIA") else {
        return false;
    };
    assert_eq!(flag, "1");
    assert_eq!(scenario(), "ui");
    let nonce = std::env::var("OMM_NATIVE_ARCHIVE_NONCE").expect("archive fixture nonce");
    Uuid::parse_str(&nonce).expect("archive fixture UUID nonce");
    let path = output("archive-nonce");
    validate_private_file(&path).unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), nonce);
    true
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
    if scenario() == "store" {
        return paths;
    }
    if matches!(scenario().as_str(), "lifecycle" | "recovery") {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap())
            .canonicalize()
            .unwrap();
        assert!(home.starts_with(&root));
        validate_private_dir(&home).unwrap();
        assert!(!home.join("Library/LaunchAgents").exists());
        assert!(
            [&paths.config, &paths.state, &paths.data, &paths.cache]
                .iter()
                .all(|p| !p.exists()),
            "lifecycle profile must be genuinely uninitialized"
        );
        return paths;
    }
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
    review::configure(&paths, &mut cfg);
    cfg.save(&paths).unwrap();
    let id = Uuid::parse_str("00000000-0000-4000-8000-000000000041").unwrap();
    let directory = paths.data.join("meetings").join(id.to_string());
    private_dir(&directory).unwrap();
    let wav = media::silent_wav(3);
    let path = directory.join("segment.wav");
    write_private_new(&path, &wav).unwrap();
    let mut meeting = SealedMeeting {
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
    let ui = scenario() == "ui";
    if ui {
        let path = meeting.directory.join("segment-2.wav");
        write_private_new(&path, &wav).unwrap();
        meeting.segments.push(AudioSegment {
            capture: None,
            path,
            duration_ms: 3000,
            sha256: hash_bytes(&wav),
        });
    }
    let mut store = Store::open(&paths).unwrap();
    store
        .set_state("capture_instance", &Uuid::new_v4())
        .unwrap();
    if archive_requested() {
        seed_archives(&paths, &mut store, &meeting, &cfg.snapshot());
    } else {
        store.enqueue(&meeting, &cfg.snapshot(), false).unwrap();
        complete_seed(&mut store, if ui { 6000 } else { 3000 });
    }
    if ui && !archive_requested() {
        let mut fixtures = Vec::new();
        for index in 1..26_u128 {
            let id = Uuid::from_u128(0x00000000000040008000000000000100 + index);
            let directory = paths.data.join("meetings").join(id.to_string());
            private_dir(&directory).unwrap();
            let path = directory.join("segment.wav");
            write_private_new(&path, &wav).unwrap();
            let title = match index {
                1 => "UI failed task fixture".into(),
                2 => "UI cancelled task fixture".into(),
                _ => format!("UI pagination fixture {index:02}"),
            };
            let item = SealedMeeting {
                session_id: id,
                title,
                directory,
                sealed_at_ms: meeting.sealed_at_ms - index as i64 * 60000,
                segments: vec![AudioSegment {
                    capture: None,
                    path,
                    duration_ms: 3000,
                    sha256: hash_bytes(&wav),
                }],
            };
            let task = store.enqueue(&item, &cfg.snapshot(), false).unwrap();
            match index {
                1 => {
                    let claim = store.claim_asr(1, 10000).unwrap().unwrap();
                    store
                        .fail(
                            &claim,
                            &OmmError::invalid("Private synthetic failed task"),
                            2,
                        )
                        .unwrap();
                }
                2 => {
                    store.desktop_cancel(task.id).unwrap();
                }
                _ => complete_seed(&mut store, 3000),
            }
            fixtures.push(serde_json::json!({"meetingId":id,"title":item.title,"taskId":task.id,"status":store.desktop_task(task.id).unwrap()["status"]}));
        }
        write_private_new(&output("ui-scenario.json"),&serde_json::to_vec_pretty(&serde_json::json!({"scenario":"ui","meetings":26,"primaryMeetingId":meeting.session_id,"primarySegments":2,"primaryDurationMs":6000,"fixtures":fixtures,"pollSeconds":180,"processSeconds":240,"webKitStore":"nonpersistent"})).unwrap()).unwrap();
    }
    review::seed(&paths);
    paths
}
fn seed_archives(
    paths: &Paths,
    store: &mut Store,
    meeting: &SealedMeeting,
    config: &ConfigSnapshot,
) {
    let mut active = meeting.clone();
    active.segments.clear();
    active.sealed_at_ms = 0;
    store.begin_segmented(&active, config).unwrap();
    for (sequence, audio) in meeting.segments.iter().enumerate() {
        store
            .admit_segment(meeting.session_id, sequence as u32, audio)
            .unwrap();
    }
    let codec = omm::adapters::asr::codec::AudioCodec::from_paths(paths).unwrap();
    let mut evidence = Vec::new();
    while let Some(claim) = store.claim_archive(100, 10000).unwrap() {
        let receipt = tauri::async_runtime::block_on(codec.archive(
            &claim.context,
            &claim.audio,
            claim.sequence,
            config.audio.opus_bitrate_kbps * 1000,
        ))
        .unwrap();
        store.commit_archive(&claim, &receipt, 101).unwrap();
        evidence.push(serde_json::json!({"input":claim.audio,"archive":receipt,"finalStoreReceipt":store.archive_for_audio(meeting.session_id,&claim.audio).unwrap()}));
    }
    while let Some(claim) = store.claim_segment(200, 10000).unwrap() {
        store
            .commit_segment(
                &claim,
                &Transcript {
                    schema_version: 1,
                    model_version: config.asr.model_version.clone(),
                    duration_ms: 3000,
                    segments: vec![TranscriptSegment {
                        source: AudioSource::Mic,
                        start_ms: 0,
                        end_ms: 3000,
                        text: "Private archive media fixture".into(),
                    }],
                },
                201,
            )
            .unwrap();
    }
    store.seal_segmented(meeting).unwrap();
    let claim = store.claim_summary(300, 10000).unwrap().unwrap();
    store
        .commit_summary(
            &claim,
            &Summary {
                schema_version: 1,
                markdown: "# Private archive media fixture".into(),
                provider: "fixture".into(),
                model: "fixture".into(),
                prompt_version: config.pi.prompt_version.clone(),
            },
            301,
        )
        .unwrap();
    let location = paths.runtime_location(&std::env::vars().collect()).unwrap();
    let guard = omm::core::runtime::RuntimeGuard::acquire(&location, &paths.state).unwrap();
    omm::application::profile::takeover(paths, &guard).unwrap();
    drop(guard);
    write_private_new(&output("archive-media.json"),&serde_json::to_vec_pretty(&serde_json::json!({"archives":evidence,"schedulerStarted":false,"hardwarePlayback":false})).unwrap()).unwrap();
}
fn complete_seed(store: &mut Store, duration_ms: u64) {
    let claim = store.claim_asr(1, 10000).unwrap().unwrap();
    store
        .commit_transcript(
            &claim,
            &Transcript {
                schema_version: 1,
                model_version: claim.context.config.asr.model_version.clone(),
                duration_ms,
                segments: vec![TranscriptSegment {
                    source: AudioSource::Mic,
                    start_ms: 0,
                    end_ms: duration_ms,
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
                markdown:
                    "# Synthetic summary\n\n## Action items\n\n- [ ] Read-only synthetic action"
                        .into(),
                provider: "fixture".into(),
                model: "fixture".into(),
                prompt_version: claim.context.config.pi.prompt_version.clone(),
            },
            4,
        )
        .unwrap();
}

pub fn builder(builder: tauri::Builder<tauri::Wry>, paths: &Paths) -> tauri::Builder<tauri::Wry> {
    if scenario() == "recovery" {
        return recovery::builder(builder, paths);
    }
    if scenario() == "lifecycle" {
        return lifecycle::builder(builder, paths);
    }
    if scenario() == "store" {
        return store::builder(builder);
    }
    let root = PathBuf::from(std::env::var_os("OMM_NATIVE_FIXTURE_ROOT").unwrap())
        .canonicalize()
        .unwrap();
    let driver = std::fs::read_to_string(root.join("driver.js")).unwrap();
    assert!(driver.len() < 128 * 1024);
    let owned = paths.clone();
    let stop = paths.clone();
    let started = Arc::new(AtomicBool::new(false));
    let helper = if archive_requested() || review::mode().as_deref() == Some("cold-recording") {
        None
    } else {
        Some(capture::start(paths))
    };
    let close = Arc::new(close::Controller::new());
    let close_events = close.clone();
    builder
        .manage(std::sync::Mutex::new(helper))
        .manage(std::sync::Mutex::new(review::Controller::new()))
        .setup(move |app| {
            store::ui_setup(app.handle());
            if scenario() == "window-acl" {
                // Same local origin and capability, but no React listener competes with this API-level probe.
                tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                    .on_web_resource_request(|_, response| { *response.body_mut() = std::borrow::Cow::Borrowed(b"<!doctype html><title>Private window ACL fixture</title>"); })
                    .build()?;
            }
            if !archive_requested() {
                let location = owned.runtime_location(&std::env::vars().collect()).unwrap();
                tauri::async_runtime::spawn(async move {
                    omm::service::daemon::run_at(owned, location)
                        .await
                        .expect("private service fixture");
                });
            }
            Ok(())
        })
        .on_window_event(move |window, event| {
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                close_events.destroyed();
            }
        })
        .on_page_load(move |view, event| {
            if event.event() != tauri::webview::PageLoadEvent::Finished
                || view.label() != "main"
                || !view.app_handle().get_webview_window("main").is_some_and(|w| crate::host::trusted(&w))
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
            let close = close.clone();
            std::thread::spawn(move || {
                let done = Arc::new(AtomicBool::new(false));
                for _ in 0..if scenario() == "ui" { 360 } else { 70 } {
                    if done.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if review::mode().is_some() {
                        if view.app_handle().state::<std::sync::Mutex<review::Controller>>().lock().unwrap().expired() {
                            done.store(true,Ordering::SeqCst);
                            review::finish(view.app_handle().clone(), stop.clone(), serde_json::json!({"ok":false,"playCalls":0,"error":"review fixture exceeded 150 seconds"}));
                            break;
                        }
                        review::poll(&view,stop.clone());
                    }
                    if scenario() == "ui" {
                        if let Some(result) = close.completion(view.app_handle()) {
                            done.store(true, Ordering::SeqCst);
                            if review::mode().is_some() {review::finish(view.app_handle().clone(),stop.clone(),result);} else {close::finish(view.app_handle().clone(), stop.clone(), result);}
                            break;
                        }
                        close.poll(&view);
                        if close.armed() {
                            continue;
                        }
                    }
                    snapshot::poll(&view);
                    let root = root.clone();
                    let paths = stop.clone();
                    let done = done.clone();
                    let app = view.app_handle().clone();
                    let close = close.clone();
                    view.eval_with_callback("JSON.stringify(window.__ommFixtureResult||null)", move |text| {
                        let Ok(serde_json::Value::String(text)) = serde_json::from_str(&text) else { return; };
                        let Ok(result) = serde_json::from_str::<serde_json::Value>(&text) else { return; };
                        if !result.is_object() || done.swap(true, Ordering::SeqCst) { return; }
                        if review::mode().is_some() {
                            review::finish(app.clone(),paths.clone(),result);
                            return;
                        }
                        if close.used() {
                            let mut result = result;
                            result["ok"] = serde_json::json!(false);
                            result["error"] = serde_json::json!("driver completed before native close observation");
                            close::finish(app.clone(), paths.clone(), result);
                            return;
                        }
                        write_private_new(
                            &root.join("bridge-result.json"),
                            &serde_json::to_vec_pretty(&serde_json::json!({"result":result,"profile":"private synthetic fixture"})).unwrap(),
                        ).unwrap();
                        let paths = paths.clone();
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if !archive_requested() {
                                let _ = omm::service::daemon::request(&paths, "shutdown", None).await;
                            }
                            app.exit(if result["ok"] == true { 0 } else { 1 });
                        });
                    }).unwrap();
                }
                if !done.load(Ordering::SeqCst) && review::mode().is_some() {
                    review::finish(view.app_handle().clone(),stop.clone(),serde_json::json!({"ok":false,"playCalls":0,"error":"review fixture exceeded 180 second completion budget"}));
                } else if !done.load(Ordering::SeqCst) && close.used() {
                    close::finish(view.app_handle().clone(), stop, serde_json::json!({"ok":false,"playCalls":0,"error":"native close fixture exceeded 180 second budget"}));
                }
            });
        })
}

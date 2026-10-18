//! Explicit command names are also enforced by Tauri ACL and the frozen schema.
#[cfg(feature = "native-bridge-fixture")]
mod bridge_fixture;
mod host;
mod startup;
#[cfg(test)]
mod window_acl;
#[tauri::command]
async fn profile_initialize(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "profile_initialize", request).await)
}
#[tauri::command]
async fn service_plan(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "service_plan", request).await)
}
#[tauri::command]
async fn service_apply(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "service_apply", request).await)
}
#[tauri::command]
async fn desktop_info(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "desktop_info", request).await)
}
#[tauri::command]
async fn meeting_list(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "meeting_list", request).await)
}
#[tauri::command]
async fn meeting_get(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "meeting_get", request).await)
}
#[tauri::command]
async fn meeting_update(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "meeting_update", request).await)
}
#[tauri::command]
async fn meeting_trash(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "meeting_trash", request).await)
}
#[tauri::command]
async fn meeting_restore(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "meeting_restore", request).await)
}
#[tauri::command]
async fn import_select(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "import_select", request).await)
}
#[tauri::command]
async fn import_commit(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "import_commit", request).await)
}
#[tauri::command]
async fn import_cancel(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "import_cancel", request).await)
}
#[tauri::command]
async fn record_status(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "record_status", request).await)
}
#[tauri::command]
async fn record_start(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "record_start", request).await)
}
#[tauri::command]
async fn record_stop(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "record_stop", request).await)
}
#[tauri::command]
async fn record_auto_set(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "record_auto_set", request).await)
}
#[tauri::command]
async fn artifact_versions(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "artifact_versions", request).await)
}
#[tauri::command]
async fn artifact_read(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "artifact_read", request).await)
}
#[tauri::command]
async fn artifact_export(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "artifact_export", request).await)
}
#[tauri::command]
async fn meeting_batch_export(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "meeting_batch_export", request).await)
}
#[tauri::command]
async fn artifact_copy(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "artifact_copy", request).await)
}
#[tauri::command]
async fn processing_enqueue(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "processing_enqueue", request).await)
}
#[tauri::command]
async fn processing_retry(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "processing_retry", request).await)
}
#[tauri::command]
async fn processing_cancel(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "processing_cancel", request).await)
}
#[tauri::command]
async fn processing_requeue(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "processing_requeue", request).await)
}
#[tauri::command]
async fn task_list(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "task_list", request).await)
}
#[tauri::command]
async fn task_get(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "task_get", request).await)
}
#[tauri::command]
async fn config_get(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "config_get", request).await)
}
#[tauri::command]
async fn config_validate(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "config_validate", request).await)
}
#[tauri::command]
async fn config_save(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "config_save", request).await)
}
#[tauri::command]
async fn diagnostics_get(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "diagnostics_get", request).await)
}
#[tauri::command]
async fn diagnostics_export(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "diagnostics_export", request).await)
}
#[tauri::command]
async fn notification_history(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "notification_history", request).await)
}
#[tauri::command]
async fn media_manifest(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "media_manifest", request).await)
}
#[tauri::command]
async fn media_open(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "media_open", request).await)
}
#[tauri::command]
async fn media_close(
    window: tauri::WebviewWindow,
    host: tauri::State<'_, host::Host>,
    request: serde_json::Value,
) -> Result<serde_json::Value, String> {
    Ok(host::invoke(window, host, "media_close", request).await)
}
fn context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}
pub fn run() {
    let startup = startup::Options::from_process().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    startup.check_support().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(3);
    });
    #[cfg(feature = "native-bridge-fixture")]
    if std::env::var_os("OMM_NATIVE_REVIEW_CLEANUP").is_some() {
        bridge_fixture::review_cleanup();
        return;
    }
    #[cfg(feature = "native-bridge-fixture")]
    if std::env::var_os("OMM_LIFECYCLE_UNKNOWN_LISTENER").is_some() {
        bridge_fixture::run_lifecycle_unknown_listener();
        return;
    }
    #[cfg(feature = "native-bridge-fixture")]
    if std::env::var_os("OMM_NATIVE_FIXTURE_HELPER").is_some() {
        bridge_fixture::capture::run();
        return;
    }
    #[cfg(not(feature = "native-bridge-fixture"))]
    let paths = omm::core::paths::Paths::from_env().expect("private profile paths");
    #[cfg(feature = "native-bridge-fixture")]
    let paths = bridge_fixture::prepare();
    let builder = tauri::Builder::default();
    #[cfg(feature = "native-bridge-fixture")]
    let builder = bridge_fixture::builder(builder, &paths);
    #[allow(unused_mut)]
    let mut context = context();
    #[cfg(feature = "native-bridge-fixture")]
    {
        // Never attach controlled fixtures to the user's persistent WebKit data store.
        context.config_mut().app.windows[0].incognito = true;
        bridge_fixture::store::configure(&mut context);
        if bridge_fixture::scenario() == "window-acl" {
            context.config_mut().app.windows[0].create = false;
        }
        context.config_mut().app.security.csp = Some(tauri::utils::config::Csp::Policy("default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src ipc: http://ipc.localhost omm-media: omm-fixture-result:; media-src omm-media:; object-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'".into()));
    }
    startup.configure(&mut context).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(3);
    });
    let app = builder
        .manage(host::Host::new(paths))
        .register_uri_scheme_protocol("omm-media", host::media_response)
        .invoke_handler(tauri::generate_handler![
            profile_initialize,
            service_plan,
            service_apply,
            desktop_info,
            meeting_list,
            meeting_get,
            meeting_update,
            meeting_trash,
            meeting_restore,
            import_select,
            import_commit,
            import_cancel,
            record_status,
            record_start,
            record_stop,
            record_auto_set,
            artifact_versions,
            artifact_read,
            artifact_export,
            meeting_batch_export,
            artifact_copy,
            processing_enqueue,
            processing_retry,
            processing_cancel,
            processing_requeue,
            task_list,
            task_get,
            config_get,
            config_validate,
            config_save,
            diagnostics_get,
            diagnostics_export,
            notification_history,
            media_manifest,
            media_open,
            media_close
        ])
        .build(context)
        .expect("desktop runtime failed");
    app.run(move |handle, event| {
        #[cfg(feature = "native-bridge-fixture")]
        if let tauri::RunEvent::ExitRequested { api, .. } = &event {
            if bridge_fixture::children::exit_blocked() {
                api.prevent_exit();
                return;
            }
        }
        if matches!(event, tauri::RunEvent::Ready) {
            startup.ready(handle);
        }
    });
}

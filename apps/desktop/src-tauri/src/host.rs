//! Only the local main webview can invoke this closed, versioned command surface.
#[cfg(test)]
#[path = "host/media_revision.rs"]
mod media_revision;
use omm::{
    application::{self as app, media::VerifiedMedia, profile},
    core::{contracts::*, paths::Paths, store::Store},
    service::daemon,
};
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Mutex};
use tauri::{Manager, WebviewWindow};
use uuid::Uuid;
struct Grant {
    media: VerifiedMedia,
    expires: i64,
}
pub struct Host {
    pub paths: Paths,
    admission: Mutex<Option<Value>>,
    grants: Mutex<HashMap<String, Grant>>,
    plans: Mutex<HashMap<String, app::lifecycle::Plan>>,
}
impl Host {
    pub fn new(paths: Paths) -> Self {
        Self {
            paths,
            admission: Mutex::new(None),
            grants: Mutex::new(HashMap::new()),
            plans: Mutex::new(HashMap::new()),
        }
    }
}
fn denied() -> OmmError {
    OmmError::new(ErrorCode::Permission, false, "untrusted desktop caller")
}
pub fn trusted(window: &WebviewWindow) -> bool {
    window.label() == "main"
        && window.url().is_ok_and(|url| {
            if cfg!(debug_assertions)
                && url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port() == Some(1420)
            {
                return true;
            }
            url.scheme() == "tauri" && url.host_str() == Some("localhost")
        })
}
impl Host {
    async fn compatibility(&self) -> Value {
        let owner = match profile::read(&self.paths) {
            Ok(Some(owner)) => owner,
            Ok(None) => return profile::incompatible("unknown", "incompatible_schema", true),
            Err(_) => return profile::incompatible("unknown", "incompatible_service", true),
        };
        let mut compatibility = match daemon::request_message_at(
            &self.paths,
            &match self.paths.runtime_location(&std::env::vars().collect()) {
                Ok(v) => v,
                Err(_) => return profile::incompatible("unknown", "incompatible_service", true),
            },
            daemon::Request {
                schema_version: 1,
                command: "desktop_capabilities_v1".into(),
                title: None,
                session_id: None,
                instance_id: None,
                desktop_request: None,
                desktop_capability: Some(profile::capability_request()),
            },
        )
        .await
        {
            Ok(value)
                if value["compatibility"]["serviceGeneration"] == owner.generation.to_string() =>
            {
                value["compatibility"].clone()
            }
            _ => match profile::offline_lock(&self.paths) {
                Ok(_lock) => profile::compatible(&owner, "offline"),
                Err(_) => {
                    return profile::incompatible("requires_switch", "incompatible_service", true)
                }
            },
        };
        let mode = compatibility["admissionMode"].as_str().unwrap_or("none");
        if compatibility["writesAllowed"] == true {
            let mut admission = self.admission.lock().unwrap();
            if !admission.as_ref().is_some_and(|a| {
                a["ownerGeneration"] == owner.generation.to_string()
                    && a["mode"] == mode
                    && a["expiresAtMs"].as_i64().unwrap_or(0)
                        > chrono::Utc::now().timestamp_millis() + 10000
            }) {
                *admission = Some(
                    json!({"ownerGeneration":owner.generation,"profileId":owner.profile_id,"stateSchemaVersion":owner.schema_version,"token":Uuid::new_v4().simple().to_string(),"expiresAtMs":chrono::Utc::now().timestamp_millis()+300000,"mode":mode}),
                );
            }
            compatibility["writeAdmission"] = admission.as_ref().unwrap().clone();
        }
        compatibility
    }
    fn verify(&self, payload: &Value) -> Result<Value> {
        let admission = self.admission.lock().unwrap().clone().ok_or_else(denied)?;
        if payload["admission"] != admission {
            return Err(denied());
        }
        let owner = profile::read(&self.paths)?.ok_or_else(denied)?;
        profile::verify_admission(&owner, &admission)?;
        Ok(admission)
    }
    async fn mutation(&self, command: &str, request: &Value) -> Result<Value> {
        let admission = self.verify(&request["payload"])?;
        if admission["mode"] == "offline" {
            if !matches!(command, "config_save") {
                return Err(OmmError::new(
                    ErrorCode::Unavailable,
                    false,
                    "compatible running service required for meeting operations",
                ));
            }
            let _service = profile::offline_lock(&self.paths)?;
            let _writer = profile::writer_lock(&self.paths)?;
            let owner = profile::read(&self.paths)?.ok_or_else(denied)?;
            profile::verify_admission(&owner, &admission)?;
            return app::configuration_journal::save(&self.paths, &owner, command, request);
        }
        let mut service_request = request.clone();
        service_request["payload"]["admission"]["token"] = json!("service_identity_bound_request");
        daemon::request_message_at(
            &self.paths,
            &self.paths.runtime_location(&std::env::vars().collect())?,
            daemon::Request {
                schema_version: 1,
                command: "desktop_dispatch_v1".into(),
                title: None,
                session_id: None,
                instance_id: Some(app::id(&admission, "ownerGeneration")?),
                desktop_capability: None,
                desktop_request: Some(json!({"command":command,"request":service_request})),
            },
        )
        .await
    }
    async fn record_status(&self) -> Result<Value> {
        let config = omm::core::config::AppConfig::load(&self.paths).unwrap_or_default();
        let compatibility = self.compatibility().await;
        let status = daemon::request(&self.paths, "status", None).await;
        let (state, meeting, error) = match status {
            Ok(value) => (
                if value["recording"].is_null() {
                    "idle"
                } else {
                    "recording"
                },
                value["recording"]["sessionId"].clone(),
                None,
            ),
            Err(error) => ("unknown", Value::Null, Some(app::error(&error))),
        };
        Ok(
            json!({"state":state,"meetingId":meeting,"startedAtMs":null,"autoRecord":config.auto_record,"recordingConsent":config.recording_consent,"compatibility":compatibility,"error":error}),
        )
    }
    async fn diagnostics(&self) -> Result<Value> {
        let compatibility = self.compatibility().await;
        let cfg = omm::core::config::AppConfig::load(&self.paths);
        let mut items = vec![
            json!({"key":"service","state":if compatibility["admissionMode"]=="service"{"ready"}else{"unknown"},"code":null,"message":if compatibility["admissionMode"]=="service"{"Compatible identity-bound service is running"}else{"Start or explicitly switch to the bundled service; legacy profiles remain read-only"},"action":"omm service start"}),
        ];
        match cfg{Ok(cfg)=>{for(key,exists)in[("capture",cfg.capture_executable.is_file()),("worker",cfg.worker_executable.is_file()),("pi",cfg.pi.executable.is_file()),("models",omm::adapters::asr::models::verify_models(&cfg.asr).is_ok())]{items.push(json!({"key":key,"state":if exists{"unknown"}else{"unavailable"},"code":null,"message":if exists{"Dependency exists; permission, authentication and model quality are not inferred"}else{"Required local dependency is missing"},"action":"See installation help"}));}},Err(_)=>items.push(json!({"key":"configuration","state":"unavailable","code":null,"message":"Initialize a private profile with omm init","action":"omm init"}))};
        Ok(
            json!({"compatibility":compatibility,"items":items,"generatedAtMs":chrono::Utc::now().timestamp_millis()}),
        )
    }
    async fn dispatch(&self, command: &str, request: &Value) -> Result<Value> {
        app::validation::request(command, request)?;
        let payload = &request["payload"];
        let value = match command {
            "desktop_info" => {
                json!({"appVersion":env!("CARGO_PKG_VERSION"),"sourceRevision":option_env!("OMM_SOURCE_REVISION").unwrap_or("development"),"contractVersion":1,"platform":"macos-arm64","minimumOs":"15.5","compatibility":self.compatibility().await})
            }
            "profile_initialize" => {
                let bundle = app::bundle::Bundle::locate()?;
                app::lifecycle::initialize(&self.paths, &bundle)?;
                app::configuration::snapshot(&self.paths, &self.compatibility().await)?
            }
            "service_plan" => {
                let bundle = app::bundle::Bundle::locate().ok();
                let (value, plan) = app::lifecycle::plan(
                    &self.paths,
                    bundle.as_ref(),
                    payload["action"].as_str().unwrap(),
                )
                .await?;
                let mut plans = self.plans.lock().unwrap();
                plans.retain(|_, p| p.expires > chrono::Utc::now().timestamp_millis());
                if let Some(plan) = plan {
                    if plans.len() >= 8 {
                        plans.clear();
                    }
                    plans.insert(plan.token.clone(), plan);
                }
                value
            }
            "service_apply" => {
                let plan = self
                    .plans
                    .lock()
                    .unwrap()
                    .remove(payload["planToken"].as_str().unwrap())
                    .ok_or_else(|| {
                        OmmError::new(
                            ErrorCode::Conflict,
                            false,
                            "service plan expired or was already consumed",
                        )
                    })?;
                let bundle = app::bundle::Bundle::locate()?;
                let mut value = app::lifecycle::apply(&self.paths, &bundle, plan).await?;
                *self.admission.lock().unwrap() = None;
                value["compatibility"] = self.compatibility().await;
                value
            }
            "config_get" => app::configuration::snapshot(&self.paths, &self.compatibility().await)?,
            "config_validate" => app::configuration::validation(&self.paths, &payload["patch"])?,
            "diagnostics_get" => self.diagnostics().await?,
            "record_status" => self.record_status().await?,
            "meeting_update" | "meeting_trash" | "meeting_restore" | "processing_enqueue"
            | "processing_retry" | "processing_cancel" | "processing_requeue" | "config_save" => {
                let mut value = self.mutation(command, request).await?;
                if command == "config_save" {
                    value["compatibility"] = self.compatibility().await;
                }
                value
            }
            "record_auto_set" => {
                self.mutation(command, request).await?;
                self.record_status().await?
            }
            "record_start" | "record_stop" => {
                let mut result = self.mutation(command, request).await?;
                result["compatibility"] = self.compatibility().await;
                result
            }
            "media_manifest" => {
                app::media::manifest(&Store::open_read_only(&self.paths)?, &self.paths, payload)?
            }
            "media_open" => {
                let store = Store::open_read_only(&self.paths)?;
                let mut list = payload.clone();
                list["cursor"] = Value::Null;
                list["pageSize"] = json!(100);
                let manifest = app::media::manifest(&store, &self.paths, &list)?;
                if manifest["revision"] != payload["expectedRevision"] {
                    return Err(OmmError::new(
                        ErrorCode::Conflict,
                        false,
                        "media manifest changed",
                    ));
                }
                let target = app::media::targets(&store, app::id(payload, "meetingId")?)?
                    .into_iter()
                    .find(|t| payload["segmentId"] == t.segment_id.to_string())
                    .ok_or_else(|| OmmError::unavailable("segment not found"))?;
                let relative = target
                    .path
                    .strip_prefix(&self.paths.data)
                    .map_err(|_| denied())?;
                let media = VerifiedMedia::open(
                    &self.paths.data,
                    relative,
                    &target.hash,
                    store.audio_read_lock()?,
                    target.mime,
                )?;
                let grant = Uuid::new_v4().simple().to_string();
                let expires = chrono::Utc::now().timestamp_millis() + 300000;
                // The grant acknowledges the manifest snapshot; VerifiedMedia retains the artifact hash/ETag.
                let result = json!({"grantId":grant,"url":format!("omm-media://localhost/{grant}"),"expiresAtMs":expires,"revision":manifest["revision"],"mime":media.mime,"byteLength":media.length});
                let mut grants = self.grants.lock().unwrap();
                grants.retain(|_, g| g.expires > chrono::Utc::now().timestamp_millis());
                if grants.len() >= 16 {
                    return Err(OmmError::unavailable(
                        "close existing media grants before opening more",
                    ));
                }
                grants.insert(grant, Grant { media, expires });
                result
            }
            "media_close" => {
                self.grants
                    .lock()
                    .unwrap()
                    .remove(payload["grantId"].as_str().unwrap_or(""));
                json!({"acknowledged":true})
            }
            "artifact_copy" => {
                let bytes = app::artifacts::text(
                    &Store::open_read_only(&self.paths)?,
                    &payload["artifact"],
                    "text",
                )?;
                if bytes.len() > 1024 * 1024 {
                    return Err(OmmError::invalid(
                        "clipboard content exceeds budget; export instead",
                    ));
                }
                #[cfg(feature = "native-bridge-fixture")]
                omm::core::paths::write_private_replace(
                    &crate::bridge_fixture::output("clipboard.txt"),
                    &bytes,
                )?;
                #[cfg(not(feature = "native-bridge-fixture"))]
                arboard::Clipboard::new()
                    .map_err(|_| OmmError::unavailable("clipboard is unavailable"))?
                    .set_text(
                        String::from_utf8(bytes)
                            .map_err(|_| OmmError::invalid("invalid text encoding"))?,
                    )
                    .map_err(|_| OmmError::unavailable("clipboard write failed"))?;
                json!({"acknowledged":true})
            }
            "artifact_export" | "meeting_batch_export" | "diagnostics_export" => {
                self.export(command, payload).await?
            }
            "import_select" => {
                let owner = profile::read(&self.paths)?.ok_or_else(|| {
                    OmmError::unavailable("initialize a private profile before importing")
                })?;
                #[cfg(feature = "native-bridge-fixture")]
                let selected = Some(crate::bridge_fixture::output("import.wav"));
                #[cfg(not(feature = "native-bridge-fixture"))]
                let selected = rfd::AsyncFileDialog::new()
                    .add_filter("PCM WAV or Ogg Opus", &["wav", "ogg", "opus"])
                    .pick_file()
                    .await
                    .map(|file| file.path().to_owned());
                match selected {
                    Some(file) => app::imports::select(&self.paths, &owner, &file).await?,
                    None => json!({"selection":null}),
                }
            }
            "import_commit" => self.mutation(command, request).await?,
            "import_cancel" => {
                let owner = profile::read(&self.paths)?.ok_or_else(denied)?;
                app::imports::cancel(
                    &self.paths,
                    &owner,
                    payload["selectionHandle"].as_str().unwrap_or(""),
                )?
            }
            _ => app::read(&Store::open_read_only(&self.paths)?, command, payload)?,
        };
        app::validation::response(command, &value)?;
        Ok(value)
    }
    async fn export(&self, command: &str, payload: &Value) -> Result<Value> {
        let bytes = if command == "diagnostics_export" {
            serde_json::to_vec_pretty(&self.diagnostics().await?)?
        } else {
            let store = Store::open_read_only(&self.paths)?;
            if command == "artifact_export" {
                app::artifacts::text(
                    &store,
                    &payload["artifact"],
                    payload["format"].as_str().unwrap_or("text"),
                )?
            } else {
                let mut rows = Vec::new();
                for id in payload["meetingIds"]
                    .as_array()
                    .ok_or_else(|| OmmError::invalid("missing meeting IDs"))?
                {
                    let meeting = Uuid::parse_str(id.as_str().unwrap_or(""))
                        .map_err(|_| OmmError::invalid("invalid meeting ID"))?;
                    let row = store.desktop_row(meeting)?;
                    let stage = payload["stage"].as_str().unwrap_or("summary");
                    let task = &row[if stage == "summary" {
                        "currentSummaryTaskId"
                    } else {
                        "currentTranscriptTaskId"
                    }];
                    if task.is_null() {
                        return Err(OmmError::unavailable(
                            "a selected meeting has no committed artifact",
                        ));
                    }
                    let artifact = json!({"meetingId":meeting,"taskId":task,"stage":stage});
                    rows.push(json!({"meetingId":meeting,"metadata":row["metadata"],"content":String::from_utf8(app::artifacts::text(&store,&artifact,payload["format"].as_str().unwrap_or("text"))?).map_err(|_|OmmError::invalid("invalid text encoding"))?}));
                }
                serde_json::to_vec_pretty(&rows)?
            }
        };
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(OmmError::invalid(
                "export exceeds 32 MiB; select fewer meetings",
            ));
        }
        #[cfg(feature = "native-bridge-fixture")]
        let selected = Some(crate::bridge_fixture::output(&format!(
            "{command}-{}.json",
            Uuid::new_v4()
        )));
        #[cfg(not(feature = "native-bridge-fixture"))]
        let selected = rfd::AsyncFileDialog::new()
            .set_file_name("omm-export.json")
            .save_file()
            .await
            .map(|file| file.path().to_owned());
        match selected {
            None => Ok(json!({"outcome":"cancelled","fileName":null})),
            Some(file) => {
                let path = file.as_path();
                if path.exists() {
                    return Err(OmmError::new(
                        ErrorCode::Conflict,
                        false,
                        "choose a new export filename; existing files are retained",
                    ));
                }
                omm::core::paths::write_private_new(path, &bytes)?;
                Ok(json!({"outcome":"saved","fileName":path.file_name().and_then(|s|s.to_str())}))
            }
        }
    }
}
pub async fn invoke(
    window: WebviewWindow,
    host: tauri::State<'_, Host>,
    command: &str,
    request: Value,
) -> Value {
    let request_id = request["requestId"].clone();
    let result = if trusted(&window) {
        host.dispatch(command, &request).await
    } else {
        Err(denied())
    };
    match result {
        Ok(value) => json!({"schemaVersion":1,"requestId":request_id,"data":value}),
        Err(error) => {
            #[cfg(feature = "native-bridge-fixture")]
            eprintln!("private synthetic fixture {command}: {error}");
            json!({"schemaVersion":1,"requestId":request_id,"error":app::error(&error)})
        }
    }
}
pub fn media_response(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let trusted = context.webview_label() == "main"
        && context
            .app_handle()
            .get_webview_window("main")
            .is_some_and(|window| trusted(&window));
    let fail = |status| {
        tauri::http::Response::builder()
            .status(status)
            .body(Vec::new())
            .unwrap()
    };
    if !trusted || request.uri().host() != Some("localhost") || request.uri().query().is_some() {
        return fail(403);
    }
    let host = context.app_handle().state::<Host>();
    let mut grants = host.grants.lock().unwrap();
    let Some(grant) = grants.get_mut(request.uri().path().trim_start_matches('/')) else {
        return fail(404);
    };
    if grant.expires <= chrono::Utc::now().timestamp_millis() {
        return fail(410);
    }
    match grant.media.response(
        request.method().as_str(),
        request.headers().get("Range").and_then(|s| s.to_str().ok()),
        request
            .headers()
            .get("If-Range")
            .and_then(|s| s.to_str().ok()),
    ) {
        Ok(value) => {
            let mut builder = tauri::http::Response::builder()
                .status(value.status)
                .header("Access-Control-Allow-Origin", "tauri://localhost");
            for (name, value) in value.headers {
                builder = builder.header(name, value);
            }
            builder.body(value.body).unwrap()
        }
        Err(_) => fail(500),
    }
}

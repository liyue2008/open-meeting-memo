use crate::{
    core::{
        config::AppConfig,
        contracts::*,
        models,
        paths::{write_private_new, Paths},
        store::Store,
    },
    service::{
        daemon,
        registration::{self, Launchctl},
    },
};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;
#[derive(Parser)]
#[command(
    name = "omm",
    version,
    about = "Local meeting audio, native transcription and Pi minutes"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub json: bool,
    #[arg(long, global = true)]
    pub non_interactive: bool,
    #[command(subcommand)]
    pub command: Commands,
}
#[derive(Subcommand)]
pub enum Commands {
    Paths,
    Init {
        #[arg(long)]
        check: bool,
    },
    Doctor,
    Notifications {
        #[command(subcommand)]
        command: NotificationCommand,
    },
    Permissions {
        #[command(subcommand)]
        command: PermissionCommand,
    },
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    Record {
        #[command(subcommand)]
        command: RecordCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Meeting {
        #[command(subcommand)]
        command: MeetingCommand,
    },
    Process {
        #[arg(required_unless_present = "task", conflicts_with = "task")]
        id: Option<Uuid>,
        #[arg(long, conflicts_with = "id")]
        task: Option<Uuid>,
        #[arg(long, value_enum, default_value = "all")]
        stage: ProcessStage,
        #[arg(long, conflicts_with = "force")]
        retry: bool,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        wait: bool,
        #[arg(long, default_value = "20m")]
        timeout: String,
    },
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    Model {
        #[command(subcommand)]
        command: ModelCommand,
    },
    Codec {
        #[command(subcommand)]
        command: CodecCommand,
    },
}
#[derive(Subcommand)]
pub enum CodecCommand {
    Check,
    Install {
        #[arg(long)]
        from: std::path::PathBuf,
    },
}
#[derive(Subcommand)]
pub enum NotificationCommand {
    History {
        #[arg(long)]
        meeting: Option<Uuid>,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    Check,
    Request,
    Test,
}
#[derive(Subcommand)]
pub enum PermissionCommand {
    Check,
    Request,
}
#[derive(Subcommand)]
pub enum ServiceCommand {
    Install,
    Start,
    Stop,
    Status,
    Uninstall,
    #[command(hide = true)]
    Run,
}
#[derive(Subcommand)]
pub enum RecordCommand {
    Start {
        #[arg(long)]
        title: Option<String>,
    },
    Stop,
    Status,
    Auto {
        #[command(subcommand)]
        command: AutoCommand,
    },
}
#[derive(Subcommand)]
pub enum AutoCommand {
    Enable {
        #[arg(long)]
        confirm_recording_consent: bool,
    },
    Disable,
}
#[derive(Subcommand)]
pub enum ConfigCommand {
    Show,
    Validate,
    Set {
        /// Typed key; audio.opus_bitrate_kbps accepts 32/48/64/96, for newly admitted segments only.
        key: String,
        value: String,
    },
}
#[derive(Subcommand)]
pub enum MeetingCommand {
    List {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    Show {
        id: Uuid,
    },
    Read {
        id: Uuid,
        #[arg(long, value_enum)]
        artifact: Artifact,
    },
}
#[derive(ValueEnum, Clone)]
pub enum Artifact {
    Transcript,
    Summary,
}
#[derive(ValueEnum, Clone, Copy)]
pub enum ProcessStage {
    Transcribe,
    Summary,
    All,
}
#[derive(Subcommand)]
pub enum TaskCommand {
    List {
        #[arg(long)]
        status: Option<String>,
    },
    Show {
        id: Uuid,
    },
    Wait {
        id: Uuid,
        #[arg(long, default_value = "10m")]
        timeout: String,
    },
}
#[derive(Subcommand)]
pub enum ModelCommand {
    List,
    Verify {
        id: String,
    },
    Download {
        id: String,
    },
    Remove {
        id: String,
        #[arg(long)]
        confirm: bool,
    },
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub schema_version: u32,
    pub ok: bool,
    pub data: Option<Value>,
    pub error: Option<OmmError>,
    pub warnings: Vec<String>,
}
pub async fn run(cli: Cli) -> i32 {
    run_with_runtime(cli, None).await
}
pub async fn run_with_runtime(
    cli: Cli,
    injected: Option<Result<crate::core::runtime::RuntimeLocation>>,
) -> i32 {
    let roots = Paths::from_env();
    let mut warnings = roots
        .as_ref()
        .map(|p| p.warnings.clone())
        .unwrap_or_default();
    let result = match roots {
        Ok(paths) => {
            let runtime =
                injected.unwrap_or_else(|| paths.runtime_location(&std::env::vars().collect()));
            match &runtime {
                Ok(location) => warnings.extend(location.warnings.clone()),
                Err(error) => warnings.push(format!("runtime rejected: {}", error.message)),
            }
            let home = std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap();
            let profile = registration::installed(&home);
            match profile.and_then(|profile| {
                if let Some(p) = profile {
                    p.check_roots(&paths)?;
                    if let Ok(location) = &runtime {
                        p.check_runtime(location)?;
                    }
                }
                Ok(())
            }) {
                Ok(()) => execute_at(&cli.command, &paths, cli.non_interactive, &runtime).await,
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    };
    let code = result.as_ref().err().map(OmmError::exit_code).unwrap_or(0);
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    let response = match result {
        Ok(data) => Response {
            schema_version: 1,
            ok: true,
            data: Some(data),
            error: None,
            warnings,
        },
        Err(error) => Response {
            schema_version: 1,
            ok: false,
            data: None,
            error: Some(error),
            warnings,
        },
    };
    if !cli.json
        && matches!(
            cli.command,
            Commands::Notifications {
                command: NotificationCommand::History { .. }
            }
        )
    {
        if let Some(entries) = response.data.as_ref().and_then(Value::as_array) {
            for entry in entries {
                println!(
                    "{} [通知发生于 {}; {}; visibility={}]",
                    entry["message"].as_str().unwrap_or("通知记录"),
                    entry["occurred_at_ms"]
                        .as_i64()
                        .and_then(chrono::DateTime::from_timestamp_millis)
                        .map(|d| d
                            .with_timezone(&chrono::Local)
                            .format("%m-%d %H:%M:%S")
                            .to_string())
                        .unwrap_or_else(|| "未知".into()),
                    entry["status"].as_str().unwrap_or("unknown"),
                    entry["visibility"].as_str().unwrap_or("unknown")
                );
            }
            return code;
        }
    }
    println!(
        "{}",
        if cli.json {
            serde_json::to_string(&response)
        } else {
            serde_json::to_string_pretty(&response)
        }
        .expect("serializable response")
    );
    code
}
fn read_store(paths: &Paths) -> Result<Store> {
    if !paths.state.join("state.sqlite").is_file() {
        return Err(OmmError::unavailable("state database not initialized"));
    }
    Store::open_read_only(paths)
}
pub async fn execute(command: &Commands, paths: &Paths, non_interactive: bool) -> Result<Value> {
    let runtime = paths.runtime_location(&std::env::vars().collect());
    execute_at(command, paths, non_interactive, &runtime).await
}
pub async fn execute_at(
    command: &Commands,
    paths: &Paths,
    non_interactive: bool,
    runtime: &Result<crate::core::runtime::RuntimeLocation>,
) -> Result<Value> {
    match command {
        Commands::Paths => Ok(
            json!({"persistent":paths,"runtime":runtime.as_ref().ok().map(|r| &r.path),"runtimeSource":runtime.as_ref().ok().map(|r| &r.source),"runtimeLifecycle":runtime.as_ref().ok().map(|r| &r.lifecycle),"runtimeError":runtime.as_ref().err(),"singleServiceScope":"one service per selected runtime and persistent STATE; explicitly isolated roots may run independently"}),
        ),
        Commands::Init { check: true } | Commands::Doctor => {
            let cfg = AppConfig::load(paths).ok();
            let capture = cfg.as_ref().is_some_and(|c| {
                c.capture_executable.is_absolute() && c.capture_executable.is_file()
            });
            let asr = cfg.as_ref().is_some_and(|c| {
                c.worker_executable.is_file()
                    && c.asr.library_path.is_file()
                    && models::assets()
                        .is_ok_and(|assets| assets.iter().all(|a| models::verify(paths, a).is_ok()))
            });
            let status = match runtime {
                Ok(location) => daemon::request_at(paths, location, "status", None)
                    .await
                    .ok(),
                Err(_) => None,
            };
            Ok(
                json!({"paths":paths,"configurationPresent":cfg.is_some(),"configurationValid":cfg.as_ref().is_some_and(|c|c.validate().is_ok()),"runtimeAvailable":runtime.is_ok(),"runtimeReady":runtime.as_ref().is_ok_and(|r|r.path.is_dir()),"runtime":runtime.as_ref().ok().map(|r| &r.path),"runtimeSource":runtime.as_ref().ok().map(|r| &r.source),"runtimeLifecycle":runtime.as_ref().ok().map(|r| &r.lifecycle),"runtimeError":runtime.as_ref().err(),"captureReady":capture,"asrReady":asr,"audioCodec":crate::core::audio_toolchain::check(paths),"serviceReady":status.is_some(),"service":status,"permissions":{"microphone":"unknown","systemAudio":"unknown","accessibility":"unknown"},"piAuthentication":"unknown; authorize dedicated CONFIG/omm/pi through Pi","componentStatus":"native_integrated; real hardware and inference unverified"}),
            )
        }
        Commands::Init { check: false } => {
            paths.create_private_dirs()?;
            let file = paths.config.join("config.json");
            if !file.exists() {
                write_private_new(&file, &serde_json::to_vec_pretty(&AppConfig::default())?)?;
            }
            let _ = Store::open(paths)?;
            Ok(
                json!({"initialized":true,"ready":false,"requires":["explicit native executable/library paths","explicit models download","dedicated Pi configuration/authentication","valid explicit XDG runtime or secure macOS best-effort replacement"]}),
            )
        }
        Commands::Notifications { command } => {
            use crate::{
                adapters::notification::MacNotificationAdapter,
                core::notifications::{NotificationAdapter, NotificationEvent},
            };
            if let NotificationCommand::History { meeting, limit } = command {
                let records = read_store(paths)?.notification_history(*meeting, *limit)?;
                return Ok(json!(records
                    .into_iter()
                    .map(|record| {
                        let message = crate::core::notifications::NotificationPayload {
                            event: record.event,
                            identity: record.identity.clone(),
                            safe_reason: record.safe_reason,
                            retry_status: record.retry_status.clone(),
                            retry_count: record.retry_count,
                        }
                        .message();
                        let mut value = json!(record);
                        value
                            .as_object_mut()
                            .unwrap()
                            .insert("message".into(), json!(message));
                        value
                    })
                    .collect::<Vec<_>>()));
            }
            let adapter = MacNotificationAdapter::default();
            if matches!(command, NotificationCommand::Check) {
                let config = AppConfig::load(paths);
                return Ok(
                    json!({"available":MacNotificationAdapter::available(),"permission":"unknown","identity":"macOS system AppleScript host; not an independently authorized OMM app","visibility":"unknown","enabled":config.as_ref().ok().map(|c|c.notifications.enabled),"configurationReadable":config.is_ok()}),
                );
            }
            if matches!(command, NotificationCommand::Request) && non_interactive {
                return Err(OmmError::new(
                    ErrorCode::InteractionRequired,
                    false,
                    "open notification settings explicitly from a desktop",
                ));
            }
            let operation = async {
                match command {
                    NotificationCommand::Request => adapter.request_settings().await,
                    NotificationCommand::Test => adapter.send(NotificationEvent::Test).await,
                    NotificationCommand::Check | NotificationCommand::History { .. } => {
                        unreachable!()
                    }
                }
            };
            let result = tokio::time::timeout(std::time::Duration::from_secs(3), operation)
                .await
                .map_err(|_| {
                    OmmError::new(ErrorCode::Timeout, false, "notification call timed out")
                });
            adapter.drain().await?;
            result??;
            Ok(
                json!({"callSucceeded":true,"settingsOpened":matches!(command,NotificationCommand::Request),"permission":"unknown","visibility":"unknown","note":"system script host controls presentation; call success is not visible delivery or authorization"}),
            )
        }
        Commands::Permissions { command } => match command {
            PermissionCommand::Check => Ok(
                json!({"microphone":"unknown","systemAudio":"unknown","accessibility":"unknown","capture":daemon::request_at(paths, runtime.as_ref().map_err(Clone::clone)?,"record_status",None).await.ok(),"note":"helper readiness is not a complete OS permission audit"}),
            ),
            PermissionCommand::Request => {
                if non_interactive || std::env::var("TERM").ok().as_deref() == Some("dumb") {
                    return Err(OmmError::new(ErrorCode::InteractionRequired,false,"system permission request needs a desktop; run this explicit command interactively"));
                }
                let status=tokio::process::Command::new("/usr/bin/open").args(["x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone","x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture","x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"]).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await.map_err(|_|OmmError::new(ErrorCode::InteractionRequired,false,"desktop permission settings unavailable"))?;
                if !status.success() {
                    return Err(OmmError::new(
                        ErrorCode::InteractionRequired,
                        false,
                        "desktop permission settings unavailable",
                    ));
                }
                Ok(
                    json!({"settingsOpened":true,"authorized":false,"note":"approve OMMCapture in System Settings; this command cannot grant permissions"}),
                )
            }
        },
        Commands::Service { command } => {
            let home = std::path::PathBuf::from(
                std::env::var_os("HOME").ok_or_else(|| OmmError::invalid("HOME missing"))?,
            );
            match command {
                ServiceCommand::Install => {
                    let _ = AppConfig::load(paths)?;
                    runtime.as_ref().map_err(Clone::clone)?;
                    registration::install_at(
                        &home,
                        paths,
                        &std::env::current_exe()?,
                        Some(runtime.as_ref().map_err(Clone::clone)?),
                        &Launchctl,
                    )
                    .await?;
                    Ok(json!({"registered":true,"roots":paths,"readyVerified":false}))
                }
                ServiceCommand::Start => {
                    runtime.as_ref().map_err(Clone::clone)?;
                    registration::start(&home, &Launchctl).await?;
                    Ok(json!({"startRequested":true,"readyVerified":false}))
                }
                ServiceCommand::Status => {
                    daemon::request_at(
                        paths,
                        runtime.as_ref().map_err(Clone::clone)?,
                        "status",
                        None,
                    )
                    .await
                }
                ServiceCommand::Stop => {
                    daemon::request_at(
                        paths,
                        runtime.as_ref().map_err(Clone::clone)?,
                        "shutdown",
                        None,
                    )
                    .await
                }
                ServiceCommand::Uninstall => {
                    registration::uninstall_at(
                        &home,
                        paths,
                        runtime.as_ref().map_err(Clone::clone)?,
                        &Launchctl,
                    )
                    .await
                }
                ServiceCommand::Run => {
                    daemon::run_at(
                        paths.clone(),
                        runtime.as_ref().map_err(Clone::clone)?.clone(),
                    )
                    .await
                }
            }
        }
        Commands::Record { command } => match command {
            RecordCommand::Start { title } => {
                daemon::request_at(
                    paths,
                    runtime.as_ref().map_err(Clone::clone)?,
                    "record_start",
                    title.clone(),
                )
                .await
            }
            RecordCommand::Stop => {
                daemon::request_at(
                    paths,
                    runtime.as_ref().map_err(Clone::clone)?,
                    "record_stop",
                    None,
                )
                .await
            }
            RecordCommand::Status => {
                daemon::request_at(
                    paths,
                    runtime.as_ref().map_err(Clone::clone)?,
                    "record_status",
                    None,
                )
                .await
            }
            RecordCommand::Auto { command } => {
                let mut cfg = AppConfig::load(paths)?;
                match command {
                    AutoCommand::Enable {
                        confirm_recording_consent: false,
                    } => {
                        return Err(OmmError::new(
                            ErrorCode::ConsentRequired,
                            false,
                            "automatic recording requires --confirm-recording-consent",
                        ))
                    }
                    AutoCommand::Enable {
                        confirm_recording_consent: true,
                    } => {
                        cfg.auto_record = true;
                        cfg.recording_consent = true;
                    }
                    AutoCommand::Disable => {
                        cfg.auto_record = false;
                    }
                }
                cfg.validate_policy()?;
                cfg.save(paths)?;
                Ok(
                    json!({"autoEnabled":cfg.auto_record,"recordingConsent":cfg.recording_consent,"serviceMustBeRunning":true}),
                )
            }
        },
        Commands::Config { command } => {
            let mut cfg = AppConfig::load(paths)?;
            match command {
                ConfigCommand::Show => Ok(
                    json!({"version":cfg.version,"asrModelVersion":cfg.asr.model_version,"pi":{"provider":cfg.pi.provider,"model":cfg.pi.model,"promptVersion":cfg.pi.prompt_version},"autoEnabled":cfg.auto_record,"notifications":cfg.notifications,"recordingConsent":cfg.recording_consent,"startDelayMs":cfg.start_delay_ms,"endGraceMs":cfg.end_grace_ms,"segmentMs":cfg.segment_ms,"audio":cfg.audio}),
                ),
                ConfigCommand::Validate => {
                    cfg.validate()?;
                    Ok(
                        json!({"valid":true,"readinessVerified":false,"note":"structural configuration validation only; doctor/model verify inspect local readiness"}),
                    )
                }
                ConfigCommand::Set { key, value } => {
                    cfg.set(key, value)?;
                    cfg.save(paths)?;
                    Ok(json!({"updatedKey":key,"configurationVersion":cfg.version}))
                }
            }
        }
        Commands::Meeting { command } => {
            let store = read_store(paths)?;
            match command {
                MeetingCommand::List {
                    from,
                    query,
                    source,
                    status,
                } => {
                    let from = from
                        .as_ref()
                        .map(|s| {
                            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                                .map_err(|_| OmmError::invalid("from must be YYYY-MM-DD"))
                        })
                        .transpose()?;
                    let tasks = store.tasks()?;
                    let status = status.as_ref().map(|s| parse_status(s)).transpose()?;
                    let mut rows = vec![];
                    for m in store.meetings()? {
                        let task = tasks
                            .iter()
                            .filter(|t| t.session_id == m.session_id)
                            .max_by_key(|t| t.attempt);
                        let record = std::fs::read(m.directory.join("meeting.json"))
                            .ok()
                            .and_then(|b| serde_json::from_slice::<daemon::Recording>(&b).ok());
                        let source_id = record.and_then(|r| r.source_id);
                        if from.is_some_and(|d| {
                            chrono::DateTime::from_timestamp_millis(m.sealed_at_ms)
                                .is_none_or(|dt| dt.date_naive() < d)
                        }) || query
                            .as_ref()
                            .is_some_and(|q| !m.title.to_lowercase().contains(&q.to_lowercase()))
                            || source
                                .as_ref()
                                .is_some_and(|q| source_id.as_deref() != Some(q))
                            || status.is_some_and(|s| task.is_none_or(|t| t.status != s))
                        {
                            continue;
                        }
                        rows.push(json!({"id":m.session_id,"title":m.title,"sealedAtMs":m.sealed_at_ms,"segments":m.segments.len(),"sourceId":source_id,"status":task.map(|t|t.status)}));
                    }
                    Ok(json!(rows))
                }
                MeetingCommand::Show { id } => {
                    let m = store
                        .meetings()?
                        .into_iter()
                        .find(|m| &m.session_id == id)
                        .ok_or_else(|| OmmError::invalid("meeting not found"))?;
                    let artifacts=store.tasks()?.into_iter().filter(|t|&t.session_id==id).map(|t|json!({"task":t,"transcript":store.artifact_path(t.id,Stage::Transcribe).ok(),"summary":store.artifact_path(t.id,Stage::Summary).ok()})).collect::<Vec<_>>();
                    let progress = if store.has_segmented_session(*id)? {
                        Some(
                            json!({"transcription":store.segment_progress(*id)?,"archives":store.archive_progress(*id)?}),
                        )
                    } else {
                        None
                    };
                    Ok(
                        json!({"meeting":m,"artifacts":artifacts,"segmentProgress":progress,"audio":store.meeting_audio(*id)?}),
                    )
                }
                MeetingCommand::Read { id, artifact } => {
                    let task = store
                        .tasks()?
                        .into_iter()
                        .filter(|t| &t.session_id == id)
                        .max_by_key(|t| t.attempt)
                        .ok_or_else(|| OmmError::invalid("meeting task not found"))?;
                    if matches!(artifact, Artifact::Transcript)
                        && store.has_segmented_session(*id)?
                        && task.attempt == 1
                        && task.stage == Stage::Transcribe
                        && !store.artifact_committed(task.id, Stage::Transcribe)?
                    {
                        let transcript = store.partial_transcript(*id)?;
                        let progress = store.segment_progress(*id)?;
                        let completed = progress.iter().filter(|s| s.status == "done").count();
                        return Ok(
                            json!({"taskId":task.id,"partial":true,"completedSegments":completed,"totalSegments":progress.len(),"notice":"转写尚未完成；这里只包含已完成片段，不是整场终稿。", "content":serde_json::to_string_pretty(&transcript)?}),
                        );
                    }
                    let bytes = store.read_artifact(
                        task.id,
                        match artifact {
                            Artifact::Transcript => Stage::Transcribe,
                            Artifact::Summary => Stage::Summary,
                        },
                    )?;
                    Ok(
                        json!({"taskId":task.id,"content":String::from_utf8(bytes).map_err(|_|OmmError::new(ErrorCode::Corrupt,false,"artifact not UTF-8"))?}),
                    )
                }
            }
        }
        Commands::Process {
            id,
            task,
            stage,
            retry,
            force,
            wait,
            timeout,
        } => {
            let _ = read_store(paths)?;
            let mut store = Store::open(paths)?;
            let task = if let Some(id) = task {
                if *force {
                    return Err(OmmError::invalid("force requires a meeting id"));
                }
                let found = store.task(*id)?;
                if *retry {
                    if store.has_segmented_session(found.session_id)? {
                        store.retry_segment_failures(found.session_id)?;
                        if matches!(
                            store.task(*id)?.status,
                            TaskStatus::Failed | TaskStatus::Blocked | TaskStatus::Interrupted
                        ) {
                            store.retry(*id)?;
                        }
                    } else {
                        store.retry(*id)?;
                    }
                }
                store.task(found.id)?
            } else {
                let id = id.ok_or_else(|| OmmError::invalid("meeting id required"))?;
                let meeting = store
                    .meetings()?
                    .into_iter()
                    .find(|m| m.session_id == id)
                    .ok_or_else(|| {
                        OmmError::unavailable("only sealed meetings can be processed")
                    })?;
                if meeting.sealed_at_ms == 0 {
                    return Err(OmmError::unavailable("active meeting is processed automatically per segment; final processing waits for stop"));
                }
                if matches!(stage, ProcessStage::Summary) {
                    let found = store
                        .tasks()?
                        .into_iter()
                        .filter(|t| t.session_id == id)
                        .max_by_key(|t| t.attempt)
                        .ok_or_else(|| {
                            OmmError::unavailable("summary requires committed transcript")
                        })?;
                    store.read_artifact(found.id, Stage::Transcribe)?;
                    if *force {
                        let config = AppConfig::load(paths)?;
                        store.force_summary(found.id, &config.version, &config.pi)?
                    } else {
                        if *retry {
                            store.retry(found.id)?;
                        }
                        store.task(found.id)?
                    }
                } else {
                    let cfg = AppConfig::load(paths)?;
                    let found = store.enqueue(&meeting, &cfg.snapshot(), *force)?;
                    if *retry {
                        store.retry(found.id)?;
                    }
                    store.task(found.id)?
                }
            };
            if matches!(stage, ProcessStage::Summary) {
                store.read_artifact(task.id, Stage::Transcribe)?;
            }
            if *wait {
                wait_task(paths, task.id, *stage, timeout).await
            } else {
                Ok(json!(task))
            }
        }
        Commands::Task { command } => match command {
            TaskCommand::List { status } => {
                let status = status.as_ref().map(|s| parse_status(s)).transpose()?;
                Ok(json!(read_store(paths)?
                    .tasks()?
                    .into_iter()
                    .filter(|t| status.is_none_or(|s| s == t.status))
                    .collect::<Vec<_>>()))
            }
            TaskCommand::Show { id } => {
                let store = read_store(paths)?;
                let task = store.task(*id)?;
                let mut result = json!(task);
                if let Ok(diagnostics) = store.asr_measurements(task.id) {
                    result
                        .as_object_mut()
                        .unwrap()
                        .insert("asrMeasurements".into(), diagnostics);
                }
                let object = result.as_object_mut().unwrap();
                object.insert("stageProgress".into(), json!(store.stage_progress(*id)?));
                object.insert("stageErrors".into(), json!(store.stage_error_events(*id)?));
                if store.has_segmented_session(task.session_id)? {
                    let object = result.as_object_mut().unwrap();
                    object.insert("segmentProgress".into(),json!({"transcription":store.segment_progress(task.session_id)?,"archives":store.archive_progress(task.session_id)?}));
                }
                Ok(result)
            }
            TaskCommand::Wait { id, timeout } => {
                wait_task(paths, *id, ProcessStage::All, timeout).await
            }
        },
        Commands::Codec { command } => match command {
            CodecCommand::Check => Ok(crate::core::audio_toolchain::check(paths)),
            CodecCommand::Install { from } => crate::core::audio_toolchain::install(paths, from),
        },
        Commands::Model { command } => match command {
            ModelCommand::List => Ok(json!(models::assets()?
                .into_iter()
                .map(|asset| {
                    let result = models::verify(paths, &asset);
                    json!({"id":asset.id,"ready":result.is_ok(),"error":result.err()})
                })
                .collect::<Vec<_>>())),
            ModelCommand::Verify { id } => {
                models::verify(paths, &models::asset(id)?)?;
                Ok(json!({"id":id,"verified":true}))
            }
            ModelCommand::Download { id } => {
                paths.create_private_dirs()?;
                let _lock = models::mutation_lock(paths)?;
                models::download(paths, &models::asset(id)?).await?;
                Ok(json!({"id":id,"installed":true,"verified":true}))
            }
            ModelCommand::Remove { id, confirm } => {
                if !confirm {
                    return Err(OmmError::new(
                        ErrorCode::ConsentRequired,
                        false,
                        "model removal requires --confirm",
                    ));
                }
                let asset = models::asset(id)?;
                let _lock = models::mutation_lock(paths)?;
                let store = read_store(paths)?;
                if store.tasks()?.iter().any(|t| {
                    matches!(
                        t.status,
                        TaskStatus::Queued | TaskStatus::Transcribing | TaskStatus::Interrupted
                    )
                }) {
                    return Err(OmmError::new(
                        ErrorCode::Conflict,
                        false,
                        "model may be in use by an active or queued transcription task",
                    ));
                }
                models::remove(paths, &asset)?;
                Ok(json!({"id":id,"removed":true,"meetingDataPreserved":true}))
            }
        },
    }
}

fn parse_status(s: &str) -> Result<TaskStatus> {
    serde_json::from_value(json!(s)).map_err(|_| OmmError::invalid("unknown task status"))
}
async fn wait_task(paths: &Paths, id: Uuid, stage: ProcessStage, timeout: &str) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + duration(timeout)?;
    loop {
        let store = read_store(paths)?;
        let task = store.task(id)?;
        if task.status == TaskStatus::Completed
            || matches!(stage, ProcessStage::Transcribe)
                && store.read_artifact(id, Stage::Transcribe).is_ok()
        {
            return Ok(json!(task));
        }
        if matches!(task.status, TaskStatus::Failed | TaskStatus::Blocked) {
            return Err(task
                .error
                .unwrap_or_else(|| OmmError::unavailable("task blocked")));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(OmmError::new(
                ErrorCode::Timeout,
                true,
                "wait timed out; persistent task was not cancelled",
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
fn duration(v: &str) -> Result<std::time::Duration> {
    let (n, multiplier) = if let Some(n) = v.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = v.strip_suffix('s') {
        (n, 1)
    } else {
        return Err(OmmError::invalid("timeout must use s or m units"));
    };
    let seconds = n
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| *n <= 86400)
        .ok_or_else(|| OmmError::invalid("timeout invalid or exceeds one day"))?;
    Ok(std::time::Duration::from_secs(seconds))
}

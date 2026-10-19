//! Real native coordination boundary with injected private platform dependencies.
use super::*;
use async_trait::async_trait;
use omm::{
    application::{
        bundle::{Bundle, Manifest},
        profile, recovery,
    },
    core::{
        config::AppConfig,
        contracts::*,
        paths::*,
        runtime::{RuntimeGuard, RuntimeLocation},
        store::hash_file,
    },
    service::registration::{self, ServiceRunner},
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::Mutex};
use std::{future::Future, pin::Pin, sync::Arc, task::Poll};
use tokio::sync::Notify;
struct Runner {
    calls: Mutex<Vec<Vec<String>>>,
    fail: bool,
}
#[async_trait]
impl ServiceRunner for Runner {
    async fn require_unloaded(&self) -> Result<()> {
        if self.fail {
            Err(OmmError::unavailable(
                "synthetic registration query failure",
            ))
        } else {
            Ok(())
        }
    }
    async fn run(&self, args: &[String]) -> Result<()> {
        self.calls.lock().unwrap().push(args.to_vec());
        if self.fail {
            Err(OmmError::unavailable("synthetic launchctl failure"))
        } else {
            Ok(())
        }
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    paths: Paths,
    runtime: RuntimeLocation,
    bundle: Bundle,
    runner: Runner,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        private_dir(&home).unwrap();
        let paths = Paths::resolve(&home, &BTreeMap::new()).unwrap();
        paths.create_private_dirs().unwrap();
        let mut cfg = AppConfig::default();
        cfg.notifications.enabled = false;
        cfg.save(&paths).unwrap();
        // Source-bound historical Store startup DDL, not current Store minus desktop tables.
        let db = paths.state.join("state.sqlite");
        write_private_new(&db, b"").unwrap();
        rusqlite::Connection::open(db)
            .unwrap()
            .execute_batch(include_str!(
                "../../../../../tests/fixtures/desktop-service-recovery-legacy-51319b7.sql"
            ))
            .unwrap();
        let rt = root.join("runtime");
        private_dir(&rt).unwrap();
        let runtime = RuntimeLocation::resolve(
            &BTreeMap::from([("XDG_RUNTIME_DIR".into(), rt.to_str().unwrap().into())]),
            || unreachable!(),
        )
        .unwrap();
        drop(RuntimeGuard::acquire(&runtime, &paths.state).unwrap());
        let old = root.join("old-omm");
        fs::write(&old, b"same semver is not trust").unwrap();
        fs::set_permissions(&old, fs::Permissions::from_mode(0o700)).unwrap();
        let resources = root.join("Resources");
        private_dir(&resources).unwrap();
        let mut files = BTreeMap::new();
        for name in [
            "bin/omm",
            "bin/omm-asr-worker",
            "OMMCapture.app/Contents/MacOS/omm-capture",
            "sherpa/libsherpa-onnx-c-api.dylib",
            "audio-toolchain/manifest.json",
        ] {
            let p = resources.join(name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"synthetic bundled resource").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
            files.insert(name.into(), hash_file(&p).unwrap());
        }
        let manifest = Manifest {
            schema_version: 1,
            version: "0.1.0".into(),
            source_revision: "0".repeat(40),
            files,
        };
        fs::write(
            resources.join("bundle-manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let bundle = Bundle::open(&resources).unwrap();
        let runner = Runner {
            calls: Mutex::new(vec![]),
            fail: false,
        };
        registration::install_at(&home, &paths, &old, Some(&runtime), &runner)
            .await
            .unwrap();
        runner.calls.lock().unwrap().clear();
        Self {
            _temp: temp,
            home,
            paths,
            runtime,
            bundle,
            runner,
        }
    }
}

async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Option<F::Output> {
    std::future::poll_fn(|cx| {
        Poll::Ready(match future.as_mut().poll(cx) {
            Poll::Ready(value) => Some(value),
            Poll::Pending => None,
        })
    })
    .await
}
impl Fixture {
    async fn lifecycle_plan(&self, action: &str) -> Result<(Value, Option<app::lifecycle::Plan>)> {
        app::lifecycle::plan_at(
            &self.paths,
            Some(&self.bundle),
            action,
            &self.home,
            &self.runtime,
            &self.runner,
        )
        .await
    }
    async fn apply(&self, plan: app::lifecycle::Plan) -> Result<Value> {
        app::lifecycle::apply_at(
            &self.paths,
            &self.bundle,
            plan,
            &self.home,
            &self.runtime,
            &self.runner,
        )
        .await
    }
}

// The controlled lock is a stand-in for the synchronous recovery hash phase. The
// operation bodies still run real plan/apply; no timing, 8MiB padding or retries.
async fn start_overlapping(
    f: Arc<Fixture>,
    host: Arc<Host>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
) -> tokio::task::JoinHandle<Result<(Value, Option<app::lifecycle::Plan>)>> {
    tokio::spawn(async move {
        host.profile_operation("service_plan", async {
            let lock = recovery::existing_lock(&f.paths.state.join("service.lock"))?;
            entered.notify_one();
            release.notified().await;
            drop(lock);
            f.lifecycle_plan("start").await
        })
        .await
    })
}

#[tokio::test]
async fn auto_start_and_manual_switch_plan_or_single_apply_do_not_contend() {
    for apply in [false, true] {
        let f = Arc::new(Fixture::new().await);
        let auto = Arc::new(Host::new(f.paths.clone()));
        // Separate Host objects for the same profile must share the process-local boundary.
        let manual = Host::new(f.paths.clone());
        let (_, issued) = manual
            .profile_operation("service_plan", f.lifecycle_plan("switch"))
            .await
            .unwrap();
        let config = fs::read(f.paths.config.join("config.json")).unwrap();
        let db = fs::read(f.paths.state.join("state.sqlite")).unwrap();
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let first = start_overlapping(f.clone(), auto, entered.clone(), release.clone()).await;
        entered.notified().await;
        let second = manual.profile_operation(
            if apply {
                "service_apply"
            } else {
                "service_plan"
            },
            async {
                if apply {
                    f.apply(issued.unwrap()).await
                } else {
                    Ok(f.lifecycle_plan("switch").await?.0)
                }
            },
        );
        tokio::pin!(second);
        let early = poll_once(second.as_mut()).await;
        release.notify_one();
        let first_value = first.await.unwrap().unwrap().0;
        assert_ne!(first_value["state"], "unknown");
        let was_pending = early.is_none();
        let value = match early {
            Some(value) => value,
            None => second.await,
        };
        assert!(was_pending, "same-profile operation entered while automatic Start held its native boundary: {value:?}");
        let value = value.unwrap();
        if apply {
            assert_eq!(value["state"], "stopped");
            assert!(profile::read(&f.paths).unwrap().is_some());
        } else {
            assert_eq!(value["canApply"], true);
            assert_eq!(value["reasons"], json!(["profile_requires_takeover"]));
        }
        assert_eq!(
            fs::read(f.paths.config.join("config.json")).unwrap(),
            config
        );
        assert_eq!(fs::read(f.paths.state.join("state.sqlite")).unwrap(), db);
        assert!(f.runner.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn adopted_stopped_info_diagnostics_and_preferences_queue_behind_start() {
    let f = Arc::new(Fixture::new().await);
    let mut host = Host::new(f.paths.clone());
    host.test_runtime = Some(f.runtime.clone());
    let host = Arc::new(host);
    let (_, plan) = host
        .profile_operation("service_plan", f.lifecycle_plan("switch"))
        .await
        .unwrap();
    host.profile_operation("service_apply", f.apply(plan.unwrap()))
        .await
        .unwrap();
    for command in [
        "desktop_info",
        "diagnostics_get",
        "config_get",
        "record_status",
    ] {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let first =
            start_overlapping(f.clone(), host.clone(), entered.clone(), release.clone()).await;
        entered.notified().await;
        let request =
            json!({"schemaVersion":1,"requestId":Uuid::new_v4().to_string(),"payload":{}});
        let mut second = Box::pin(host.dispatch(command, &request));
        let early = poll_once(second.as_mut()).await;
        release.notify_one();
        let start = first.await.unwrap().unwrap().0;
        assert_eq!(start["canApply"], true);
        let was_pending = early.is_none();
        let value = match early {
            Some(value) => value,
            None => second.await,
        };
        assert!(
            was_pending,
            "{command} escaped native dispatch boundary: {value:?}"
        );
        assert_eq!(value.unwrap()["compatibility"]["state"], "compatible");
    }
    assert!(f.runner.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bounded_wait_cancel_error_panic_and_other_profiles_release_without_operations() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = Fixture::new().await;
    let other = Fixture::new().await;
    let host = Arc::new(Host::new(f.paths.clone()));
    let other_host = Host::new(other.paths.clone());
    let held = app::local_coordination::enter(&f.paths, std::time::Duration::from_secs(1))
        .await
        .unwrap();
    let calls = AtomicUsize::new(0);
    let mut cancelled = Box::pin(host.profile_operation("service_apply", async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));
    assert!(poll_once(cancelled.as_mut()).await.is_none());
    drop(cancelled);
    let timeout = host
        .profile_operation_with_budget("service_apply", std::time::Duration::ZERO, async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(timeout.code, ErrorCode::Conflict);
    assert!(timeout.retryable);
    assert!(timeout.message.contains("local profile"));
    // Exercise a real executor timer as well as the zero-budget boundary; no paused clock.
    assert_eq!(
        host.profile_operation_with_budget(
            "service_apply",
            std::time::Duration::from_millis(2),
            async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::Conflict
    );
    other_host
        .profile_operation_with_budget("service_plan", std::time::Duration::ZERO, async { Ok(()) })
        .await
        .unwrap();
    // Long dialogs/audio streams and pure meeting reads are not profile probe critical sections.
    for command in [
        "media_open",
        "meeting_list",
        "import_select",
        "artifact_export",
        "diagnostics_export",
    ] {
        host.profile_operation_with_budget(command, std::time::Duration::ZERO, async { Ok(()) })
            .await
            .unwrap();
    }
    for command in [
        "profile_initialize",
        "service_plan",
        "service_apply",
        "desktop_info",
        "diagnostics_get",
        "config_get",
        "config_save",
        "record_status",
        "record_start",
        "record_stop",
        "record_auto_set",
    ] {
        let busy = host
            .profile_operation_with_budget(command, std::time::Duration::ZERO, async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap_err();
        assert_eq!(busy.code, ErrorCode::Conflict, "{command}");
    }
    drop(held);
    host.profile_operation("service_plan", async { Ok(()) })
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let failure = host
        .profile_operation("service_apply", async {
            Err::<(), _>(OmmError::invalid("synthetic failure"))
        })
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::InvalidInput);
    let panic_host = host.clone();
    let panic_task = tokio::spawn(async move {
        panic_host
            .profile_operation("service_plan", async {
                panic!("synthetic unwind");
                #[allow(unreachable_code)]
                Ok(())
            })
            .await
    });
    assert!(panic_task.await.unwrap_err().is_panic());
    let entered = Arc::new(Notify::new());
    let cancel_host = host.clone();
    let inside = entered.clone();
    let active = tokio::spawn(async move {
        cancel_host
            .profile_operation("service_apply", async {
                inside.notify_one();
                std::future::pending::<Result<()>>().await
            })
            .await
    });
    entered.notified().await;
    active.abort();
    assert!(active.await.unwrap_err().is_cancelled());
    for command in [
        "profile_initialize",
        "service_plan",
        "service_apply",
        "desktop_info",
        "diagnostics_get",
        "config_get",
        "config_save",
        "record_status",
        "record_start",
        "record_stop",
        "record_auto_set",
    ] {
        host.profile_operation_with_budget(command, std::time::Duration::ZERO, async { Ok(()) })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn queued_confirmation_rechecks_expiry_and_does_not_adopt() {
    let f = Fixture::new().await;
    let host = Host::new(f.paths.clone());
    let (_, plan) = host
        .profile_operation("service_plan", f.lifecycle_plan("switch"))
        .await
        .unwrap();
    let mut plan = plan.unwrap();
    // Inject only this plan's deadline; production still signs 60 seconds. No fake runtime clock.
    plan.expires = chrono::Utc::now().timestamp_millis() + 5;
    let expires = plan.expires;
    let held = app::local_coordination::enter(&f.paths, std::time::Duration::from_secs(1))
        .await
        .unwrap();
    let mut queued = Box::pin(host.profile_operation("service_apply", f.apply(plan)));
    assert!(poll_once(queued.as_mut()).await.is_none());
    while chrono::Utc::now().timestamp_millis() <= expires {
        tokio::task::yield_now().await;
    }
    drop(held);
    assert_eq!(queued.await.unwrap_err().code, ErrorCode::Conflict);
    assert!(profile::read(&f.paths).unwrap().is_none());
    assert!(f.runner.calls.lock().unwrap().is_empty());
    host.profile_operation("service_plan", f.lifecycle_plan("switch"))
        .await
        .unwrap();
}

#[tokio::test]
async fn real_changes_remain_stale_through_native_boundary() {
    for change in ["config", "registration", "database", "generation"] {
        let f = Fixture::new().await;
        let host = Host::new(f.paths.clone());
        if change == "generation" {
            let (_, p) = host
                .profile_operation("service_plan", f.lifecycle_plan("switch"))
                .await
                .unwrap();
            host.profile_operation("service_apply", f.apply(p.unwrap()))
                .await
                .unwrap();
        }
        let action = if change == "generation" {
            "start"
        } else {
            "switch"
        };
        let (_, plan) = host
            .profile_operation("service_plan", f.lifecycle_plan(action))
            .await
            .unwrap();
        match change {
            "config" => {
                let mut cfg = AppConfig::load(&f.paths).unwrap();
                cfg.poll_ms += 1;
                cfg.save(&f.paths).unwrap();
            }
            "registration" => {
                let p = f
                    .home
                    .join("Library/LaunchAgents/org.openmeetingmemo.service.plist");
                let mut b = fs::read(&p).unwrap();
                b.push(b'\n');
                fs::write(p, b).unwrap();
            }
            "database" => {
                rusqlite::Connection::open(f.paths.state.join("state.sqlite")).unwrap().execute("INSERT INTO settings(key,document) VALUES('synthetic_external_change','true')",[]).unwrap();
            }
            "generation" => {
                let mut owner = profile::read(&f.paths).unwrap().unwrap();
                owner.generation = Uuid::new_v4();
                write_private_replace(
                    &f.paths.state.join("desktop-owner.json"),
                    &serde_json::to_vec(&owner).unwrap(),
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            host.profile_operation("service_apply", f.apply(plan.unwrap()))
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict,
            "{change}"
        );
        assert!(f.runner.calls.lock().unwrap().is_empty(), "{change}");
    }
}

// Run alone: an unrelated concurrent fork can temporarily inherit another test's
// CLOEXEC OS lock before exec. This is not a change to production lock semantics.
#[tokio::test]
#[ignore = "isolated external-process fixture; run this exact test separately"]
async fn external_process_os_lock_is_not_hidden_by_local_coordination() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    struct ChildLock(std::process::Child);
    impl Drop for ChildLock {
        fn drop(&mut self) {
            drop(self.0.stdin.take());
            assert!(self.0.wait().unwrap().success());
        }
    }
    let f = Fixture::new().await;
    let host = Host::new(f.paths.clone());
    let mut child=ChildLock(Command::new("python3").args(["-c","import fcntl,sys; f=open(sys.argv[1],'r'); fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB); print('locked',flush=True); sys.stdin.read()"])
        .arg(f.paths.state.join("service.lock")).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap());
    let mut ready = String::new();
    BufReader::new(child.0.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    assert_eq!(ready, "locked\n");
    let (value, plan) = host
        .profile_operation("service_plan", f.lifecycle_plan("switch"))
        .await
        .unwrap();
    assert_eq!(value["canApply"], false);
    assert!(plan.is_none());
    assert_eq!(value["reasons"], json!(["registration_review_required"]));
    drop(child); // Normal pipe-close + wait, not PID killing.
    let (value, _) = host
        .profile_operation("service_plan", f.lifecycle_plan("switch"))
        .await
        .unwrap();
    assert_eq!(value["canApply"], true);
}

#[tokio::test]
async fn initialization_keeps_stable_gate_and_rejects_alias_without_writes() {
    let root = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let root = root.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("new-home"), &BTreeMap::new()).unwrap();
    let host = Host::new(paths.clone());
    let held = app::local_coordination::enter(&paths, std::time::Duration::from_secs(1))
        .await
        .unwrap();
    assert!(!paths.state.exists());
    paths.create_private_dirs().unwrap();
    let failure = host
        .profile_operation_with_budget("profile_initialize", std::time::Duration::ZERO, async {
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::Conflict);
    drop(held);
    host.profile_operation("profile_initialize", async { Ok(()) })
        .await
        .unwrap();
    let alias = root.join("alias");
    std::os::unix::fs::symlink(&paths.state, &alias).unwrap();
    let mut aliased = paths;
    aliased.state = alias;
    assert!(
        app::local_coordination::enter(&aliased, std::time::Duration::ZERO)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires this revision's private current CLI via OMM_COORDINATION_TEST_CLI"]
async fn native_boundary_covers_actual_private_start_and_drained_stop() {
    let mut f = Fixture::new().await;
    let mut cfg = AppConfig::load(&f.paths).unwrap();
    cfg.auto_record = false;
    cfg.recording_consent = false;
    cfg.notifications.enabled = false;
    cfg.poll_ms = 60000;
    cfg.save(&f.paths).unwrap();
    let cli = f.bundle.resources.join("bin/omm");
    fs::copy(
        std::env::var_os("OMM_COORDINATION_TEST_CLI").expect("explicit private current CLI"),
        &cli,
    )
    .unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    let manifest = f.bundle.resources.join("bundle-manifest.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["files"]["bin/omm"] = json!(hash_file(&cli).unwrap());
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    f.bundle = Bundle::open(&f.bundle.resources).unwrap();
    let f = Arc::new(f);
    let host = Arc::new(Host::new(f.paths.clone()));
    let (_, plan) = host
        .profile_operation("service_plan", f.lifecycle_plan("switch"))
        .await
        .unwrap();
    assert_eq!(
        host.profile_operation("service_apply", f.apply(plan.unwrap()))
            .await
            .unwrap()["state"],
        "stopped"
    );
    let (_, plan) = host
        .profile_operation("service_plan", f.lifecycle_plan("start"))
        .await
        .unwrap();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let first = {
        let f = f.clone();
        let host = host.clone();
        let entered = entered.clone();
        let release = release.clone();
        tokio::spawn(async move {
            host.profile_operation("service_apply", async {
                entered.notify_one();
                release.notified().await;
                f.apply(plan.unwrap()).await
            })
            .await
        })
    };
    entered.notified().await;
    let mut auto = Box::pin(host.profile_operation("service_plan", f.lifecycle_plan("start")));
    let early = poll_once(auto.as_mut()).await;
    release.notify_one();
    let started = first.await.unwrap();
    let was_pending = early.is_none();
    let automatic = match early {
        Some(value) => value,
        None => auto.await,
    };
    // Always drain our verified child before asserting the operation results.
    let owner = omm::core::runtime::verify_active(&f.runtime.path, &f.paths.state).ok();
    let mut stopped = None;
    if let Some(owner) = &owner {
        if app::lifecycle::owned_service(owner, &f.bundle).unwrap_or(false) {
            if let Ok((_, Some(plan))) = host
                .profile_operation("service_plan", f.lifecycle_plan("stop"))
                .await
            {
                stopped = Some(host.profile_operation("service_apply", f.apply(plan)).await);
            }
            if omm::core::runtime::owner_is_live(owner) {
                let _ = daemon::request_message_at(
                    &f.paths,
                    &f.runtime,
                    daemon::Request {
                        schema_version: 1,
                        command: "shutdown".into(),
                        instance_id: Some(owner.instance_id),
                        title: None,
                        session_id: None,
                        desktop_request: None,
                        desktop_capability: None,
                    },
                )
                .await;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while omm::core::runtime::owner_is_live(owner) && std::time::Instant::now() < deadline {
                tokio::task::yield_now().await;
            }
        }
    }
    assert!(
        was_pending,
        "background Start entered during explicit Start"
    );
    assert_eq!(started.unwrap()["state"], "running");
    let (value, plan) = automatic.unwrap();
    assert_eq!(value["state"], "running");
    assert_eq!(value["canApply"], false);
    assert!(plan.is_none());
    assert_eq!(stopped.unwrap().unwrap()["state"], "stopped");
    assert!(!omm::core::runtime::owner_is_live(&owner.unwrap()));
    assert!(f.runner.calls.lock().unwrap().is_empty());
}

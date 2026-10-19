//! Stopped legacy takeover uses only synthetic profiles, fake registration and no notifications.
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
        store::{hash_file, Store},
    },
    service::registration::{self, ServiceRunner},
};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::Mutex};
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
                "fixtures/desktop-service-recovery-legacy-51319b7.sql"
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
    fn plan(&self) -> Result<recovery::RecoverySnapshot> {
        recovery::inspect(&self.paths, &self.home, &self.runtime, &self.bundle)
    }
    fn plist(&self) -> PathBuf {
        self.home
            .join("Library/LaunchAgents/org.openmeetingmemo.service.plist")
    }
}
#[tokio::test]
async fn desktop_service_recovery_stopped_adoption_preserves_config_and_does_not_start_old_or_new_cli(
) {
    let f = Fixture::new().await;
    let config = fs::read(f.paths.config.join("config.json")).unwrap();
    let db = fs::read(f.paths.state.join("state.sqlite")).unwrap();
    let before = fs::read(f.plist()).unwrap();
    let plan = f.plan().unwrap();
    assert!(profile::read(&f.paths).unwrap().is_none());
    assert_eq!(fs::read(f.plist()).unwrap(), before);
    assert!(f.runner.calls.lock().unwrap().is_empty());
    recovery::adopt(&f.paths, &f.home, &f.runtime, &f.bundle, &plan, &f.runner)
        .await
        .unwrap();
    assert!(profile::read(&f.paths).unwrap().is_some());
    assert_eq!(
        config,
        fs::read(f.paths.config.join("config.json")).unwrap()
    );
    assert_eq!(db, fs::read(f.paths.state.join("state.sqlite")).unwrap());
    let plist = fs::read_to_string(f.plist()).unwrap();
    assert!(plist.contains(f.bundle.cli().unwrap().to_str().unwrap()));
    assert!(f.runner.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn desktop_service_recovery_rejects_active_or_unknown_work_and_new_schema() {
    for kind in [
        "recording",
        "capture_helper_expected",
        "unsealed",
        "lease",
        "new_schema",
        "unknown_table",
        "orphan",
        "wal",
    ] {
        let f = Fixture::new().await;
        match kind {
            "recording" => Store::open(&f.paths)
                .unwrap()
                .set_state("recording", &serde_json::json!({}))
                .unwrap(),
            "capture_helper_expected" => Store::open(&f.paths)
                .unwrap()
                .set_state("capture_helper_expected", &true)
                .unwrap(),
            "unsealed" => {
                let c = rusqlite::Connection::open(f.paths.state.join("state.sqlite")).unwrap();
                c.execute_batch("PRAGMA foreign_keys=OFF; INSERT INTO segmented_sessions VALUES('unknown','unknown',0)").unwrap();
            }
            "lease" => {
                let c = rusqlite::Connection::open(f.paths.state.join("state.sqlite")).unwrap();
                c.execute_batch("PRAGMA foreign_keys=OFF; INSERT INTO segment_archives(session,sequence,lease) VALUES('unknown',0,'unknown')").unwrap();
            }
            "new_schema" => {
                rusqlite::Connection::open(f.paths.state.join("state.sqlite"))
                    .unwrap()
                    .execute_batch("PRAGMA user_version=100")
                    .unwrap();
            }
            "unknown_table" => {
                rusqlite::Connection::open(f.paths.state.join("state.sqlite"))
                    .unwrap()
                    .execute_batch("CREATE TABLE future_unknown(value TEXT)")
                    .unwrap();
            }
            "orphan" => {
                private_dir(&f.paths.cache.join("pi")).unwrap();
                private_dir(&f.paths.cache.join("pi/unknown-workspace")).unwrap();
            }
            "wal" => write_private_new(&f.paths.state.join("state.sqlite-wal"), b"do not ignore")
                .unwrap(),
            _ => unreachable!(),
        };
        let before = fs::read(f.plist()).unwrap();
        assert!(f.plan().is_err(), "{kind}");
        assert!(profile::read(&f.paths).unwrap().is_none());
        assert_eq!(fs::read(f.plist()).unwrap(), before);
    }
}
#[tokio::test]
async fn desktop_service_recovery_rejects_generation_changes_and_failed_registration_keeps_owner_absent(
) {
    let f = Fixture::new().await;
    let plan = f.plan().unwrap();
    let original = fs::read(f.plist()).unwrap();
    let fail = Runner {
        calls: Mutex::new(vec![]),
        fail: true,
    };
    assert!(
        recovery::adopt(&f.paths, &f.home, &f.runtime, &f.bundle, &plan, &fail)
            .await
            .is_err()
    );
    assert!(profile::read(&f.paths).unwrap().is_none());
    assert_eq!(fs::read(f.plist()).unwrap(), original);
    let plan = f.plan().unwrap();
    Store::open(&f.paths)
        .unwrap()
        .set_state("changed", &true)
        .unwrap();
    assert!(
        recovery::adopt(&f.paths, &f.home, &f.runtime, &f.bundle, &plan, &f.runner)
            .await
            .is_err()
    );
    assert!(f.runner.calls.lock().unwrap().is_empty());
}
#[tokio::test]
async fn desktop_service_recovery_live_owner_same_version_never_becomes_adoptable() {
    let f = Fixture::new().await;
    let guard = RuntimeGuard::acquire(&f.runtime, &f.paths.state).unwrap();
    assert!(f.plan().is_err());
    assert!(f.runner.calls.lock().unwrap().is_empty());
    drop(guard);
}

fn tree(path: &std::path::Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    if path.is_dir() {
        for item in fs::read_dir(path).unwrap() {
            let p = item.unwrap().path();
            if p.is_dir() {
                files.extend(tree(&p));
            } else {
                files.insert(p.clone(), fs::read(p).unwrap());
            }
        }
    }
    files
}
#[tokio::test]
async fn desktop_service_recovery_lifecycle_plan_is_read_only_expired_and_repeated_confirmations_refuse(
) {
    use omm::application::lifecycle;
    let f = Fixture::new().await;
    let before = tree(&f.home);
    let (value, plan) = lifecycle::plan_at(
        &f.paths,
        Some(&f.bundle),
        "switch",
        &f.home,
        &f.runtime,
        &f.runner,
    )
    .await
    .unwrap();
    assert_eq!(value["state"], "stopped");
    assert_eq!(value["canApply"], true);
    assert_eq!(value["reasons"][0], "profile_requires_takeover");
    assert_eq!(value["effects"].as_array().unwrap().len(), 4);
    assert_eq!(before, tree(&f.home));
    let plan = plan.unwrap();
    let mut expired = plan.clone();
    expired.expires = 0;
    assert!(
        lifecycle::apply_at(&f.paths, &f.bundle, expired, &f.home, &f.runtime, &f.runner)
            .await
            .is_err()
    );
    let result = lifecycle::apply_at(
        &f.paths,
        &f.bundle,
        plan.clone(),
        &f.home,
        &f.runtime,
        &f.runner,
    )
    .await
    .unwrap();
    assert_eq!(result["state"], "stopped");
    assert!(
        lifecycle::apply_at(&f.paths, &f.bundle, plan, &f.home, &f.runtime, &f.runner)
            .await
            .is_err()
    );
    let (next, _) = lifecycle::plan_at(
        &f.paths,
        Some(&f.bundle),
        "start",
        &f.home,
        &f.runtime,
        &f.runner,
    )
    .await
    .unwrap();
    assert_eq!(next["canApply"], true);
}
#[tokio::test]
async fn desktop_service_recovery_unverifiable_live_plan_never_contacts_legacy_socket() {
    use omm::application::lifecycle;
    let f = Fixture::new().await;
    let guard = RuntimeGuard::acquire(&f.runtime, &f.paths.state).unwrap();
    let listener = tokio::net::UnixListener::bind(f.runtime.path.join("service.sock")).unwrap();
    fs::set_permissions(
        f.runtime.path.join("service.sock"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let (value, plan) = lifecycle::plan_at(
        &f.paths,
        Some(&f.bundle),
        "switch",
        &f.home,
        &f.runtime,
        &f.runner,
    )
    .await
    .unwrap();
    assert!(plan.is_none());
    assert_eq!(value["canApply"], false);
    assert!(value["reasons"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("external_stop_required")));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    drop(guard);
}
#[tokio::test]
async fn desktop_service_recovery_preferences_and_status_require_explicit_fields_and_generation() {
    use omm::application::observation;
    let f = Fixture::new().await;
    let generation = uuid::Uuid::new_v4();
    let compatibility =
        serde_json::json!({"admissionMode":"service","serviceGeneration":generation});
    for status in [
        serde_json::json!({}),
        serde_json::json!({"running":true,"serviceInstanceId":generation}),
        serde_json::json!({"running":true,"recording":null,"serviceInstanceId":uuid::Uuid::new_v4()}),
        serde_json::json!({"running":true,"recording":{},"serviceInstanceId":generation}),
    ] {
        assert_eq!(
            observation::record_status(&f.paths, compatibility.clone(), Some(Ok(status)))["state"],
            "unknown"
        );
    }
    let idle = observation::record_status(
        &f.paths,
        compatibility.clone(),
        Some(Ok(
            serde_json::json!({"running":true,"recording":null,"serviceInstanceId":generation}),
        )),
    );
    assert_eq!(idle["state"], "idle");
    let path = f.paths.config.join("config.json");
    write_private_replace(&path, b"{}").unwrap();
    let result = observation::record_status(&f.paths, compatibility.clone(), None);
    assert_eq!(result["state"], "unknown");
    assert!(result["autoRecord"].is_null());
    assert!(result["recordingConsent"].is_null());
    let snapshot =
        omm::application::configuration::snapshot(&f.paths, &serde_json::json!({})).unwrap();
    assert!(snapshot["autoRecord"].is_null());
    assert!(snapshot["recordingConsent"].is_null());
    write_private_replace(&path, b"{bad json").unwrap();
    assert_eq!(observation::recording_preferences(&f.paths), (None, None));
    fs::remove_file(path).unwrap();
    assert_eq!(observation::recording_preferences(&f.paths), (None, None));
}
enum Fault {
    RegistrationChanged,
    OwnerAppeared,
}
struct FaultRunner {
    plist: PathBuf,
    owner: PathBuf,
    fault: Fault,
    checks: std::sync::atomic::AtomicUsize,
}
#[async_trait]
impl ServiceRunner for FaultRunner {
    async fn run(&self, args: &[String]) -> Result<()> {
        panic!("recovery must not execute launchctl mutations: {args:?}");
    }
    async fn require_unloaded(&self) -> Result<()> {
        if self
            .checks
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            == 0
        {
            return Ok(());
        }
        match self.fault {
            Fault::RegistrationChanged => {
                write_private_replace(&self.plist, b"concurrent registration")?
            }
            Fault::OwnerAppeared => write_private_new(&self.owner, b"concurrent owner")?,
        }
        Ok(())
    }
}
#[tokio::test]
async fn desktop_service_recovery_publication_cas_and_owner_failures_preserve_concurrent_files() {
    for fault in [Fault::RegistrationChanged, Fault::OwnerAppeared] {
        let f = Fixture::new().await;
        let before = fs::read(f.plist()).unwrap();
        let owner = f.paths.state.join("desktop-owner.json");
        let plan = f.plan().unwrap();
        let runner = FaultRunner {
            plist: f.plist(),
            owner: owner.clone(),
            fault,
            checks: std::sync::atomic::AtomicUsize::new(0),
        };
        let error = recovery::adopt(&f.paths, &f.home, &f.runtime, &f.bundle, &plan, &runner)
            .await
            .unwrap_err();
        match runner.fault {
            Fault::RegistrationChanged => {
                assert_eq!(fs::read(f.plist()).unwrap(), b"concurrent registration");
                assert!(!owner.exists());
                assert!(error.message.contains("unloaded"));
            }
            Fault::OwnerAppeared => {
                assert_eq!(fs::read(f.plist()).unwrap(), before);
                assert_eq!(fs::read(owner).unwrap(), b"concurrent owner");
                assert!(error.message.contains("unloaded"));
            }
        }
    }
}
#[tokio::test]
async fn desktop_service_recovery_adopt_then_explicit_start_is_usable_and_second_start_cannot_duplicate(
) {
    use omm::application::lifecycle;
    let mut f = Fixture::new().await;
    let mut cfg = AppConfig::load(&f.paths).unwrap();
    cfg.auto_record = false;
    cfg.recording_consent = false;
    cfg.notifications.enabled = false;
    cfg.poll_ms = 60000;
    cfg.save(&f.paths).unwrap();
    let (_, adopt) = lifecycle::plan_at(
        &f.paths,
        Some(&f.bundle),
        "switch",
        &f.home,
        &f.runtime,
        &f.runner,
    )
    .await
    .unwrap();
    lifecycle::apply_at(
        &f.paths,
        &f.bundle,
        adopt.unwrap(),
        &f.home,
        &f.runtime,
        &f.runner,
    )
    .await
    .unwrap();
    let owner_before = fs::read(f.paths.state.join("desktop-owner.json")).unwrap();
    let (_, failed) = lifecycle::plan_at(
        &f.paths,
        Some(&f.bundle),
        "start",
        &f.home,
        &f.runtime,
        &f.runner,
    )
    .await
    .unwrap();
    assert!(lifecycle::apply_at(
        &f.paths,
        &f.bundle,
        failed.unwrap(),
        &f.home,
        &f.runtime,
        &f.runner
    )
    .await
    .is_err());
    assert_eq!(
        fs::read(f.paths.state.join("desktop-owner.json")).unwrap(),
        owner_before
    );
    let cli = f.bundle.resources.join("bin/omm");
    fs::copy(env!("CARGO_BIN_EXE_omm"), &cli).unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    let manifest = f.bundle.resources.join("bundle-manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    value["files"]["bin/omm"] = serde_json::json!(hash_file(&cli).unwrap());
    fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
    f.bundle = Bundle::open(&f.bundle.resources).unwrap();
    for round in 0..2 {
        let task = if round == 1 {
            Some(record_modern_asr_measurement(&f))
        } else {
            None
        };
        let measurements_before = task.map(|id| {
            Store::open_read_only(&f.paths)
                .unwrap()
                .asr_measurements(id)
                .unwrap()
        });
        let before = recovery_bytes(&f);
        let (_, start) = lifecycle::plan_at(
            &f.paths,
            Some(&f.bundle),
            "start",
            &f.home,
            &f.runtime,
            &f.runner,
        )
        .await
        .unwrap();
        assert!(
            start.is_some(),
            "normal ASR tables must pass shared stopped_start_snapshot"
        );
        assert_eq!(before, recovery_bytes(&f));
        if task.is_some() {
            println!(
                "modern ASR restart plan: preserved config/database/sidecars {}",
                serde_json::to_string(&before).unwrap()
            );
        }
        let started = lifecycle::apply_at(
            &f.paths,
            &f.bundle,
            start.unwrap(),
            &f.home,
            &f.runtime,
            &f.runner,
        )
        .await;
        let owner = omm::core::runtime::verify_active(&f.runtime.path, &f.paths.state).ok();
        let duplicate = lifecycle::plan_at(
            &f.paths,
            Some(&f.bundle),
            "start",
            &f.home,
            &f.runtime,
            &f.runner,
        )
        .await;
        let mut stopped = None;
        if let Some(owner) = &owner {
            if lifecycle::owned_service(owner, &f.bundle).unwrap_or(false) {
                let plan = lifecycle::plan_at(
                    &f.paths,
                    Some(&f.bundle),
                    "stop",
                    &f.home,
                    &f.runtime,
                    &f.runner,
                )
                .await;
                if let Ok((_, Some(plan))) = plan {
                    stopped = Some(
                        lifecycle::apply_at(
                            &f.paths, &f.bundle, plan, &f.home, &f.runtime, &f.runner,
                        )
                        .await,
                    );
                }
                if omm::core::runtime::owner_is_live(owner) {
                    let _ = omm::service::daemon::request_message_at(
                        &f.paths,
                        &f.runtime,
                        omm::service::daemon::Request {
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
                for _ in 0..100 {
                    if !omm::core::runtime::owner_is_live(owner) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
        }
        assert_eq!(started.unwrap()["state"], "running");
        let (duplicate, duplicate_plan) = duplicate.unwrap();
        assert_eq!(duplicate["canApply"], false);
        assert!(duplicate_plan.is_none());
        assert_eq!(stopped.unwrap().unwrap()["state"], "stopped");
        assert!(!omm::core::runtime::owner_is_live(&owner.unwrap()));
        assert!(f.runner.calls.lock().unwrap().is_empty());
        if let Some(task) = task {
            assert_eq!(
                Some(
                    Store::open_read_only(&f.paths)
                        .unwrap()
                        .asr_measurements(task)
                        .unwrap()
                ),
                measurements_before
            );
            assert_eq!(before["config"], recovery_bytes(&f)["config"]);
        }
    }
}
#[tokio::test]
async fn desktop_service_recovery_durable_status_read_does_not_initialize_or_migrate_store() {
    let f = Fixture::new().await;
    let guard = RuntimeGuard::acquire(&f.runtime, &f.paths.state).unwrap();
    let _listener = tokio::net::UnixListener::bind(f.runtime.path.join("service.sock")).unwrap();
    fs::set_permissions(
        f.runtime.path.join("service.sock"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let before = fs::read(f.paths.state.join("state.sqlite")).unwrap();
    let status =
        omm::application::observation::service_status(&f.paths, &f.runtime, &guard.owner).unwrap();
    assert!(status["recording"].is_null());
    assert_eq!(
        before,
        fs::read(f.paths.state.join("state.sqlite")).unwrap()
    );
    let conn = rusqlite::Connection::open_with_flags(
        f.paths.state.join("state.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name LIKE 'desktop_%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    conn.close().unwrap();
    drop(guard);
}
#[test]
fn desktop_service_recovery_unloaded_receipt_is_exact_and_bounded() {
    let expected =
        b"Could not find service \"org.openmeetingmemo.service\" in domain for user gui: 501\n";
    assert!(registration::recovery::require_unloaded_output(501, Some(113), expected).is_ok());
    for (code, text) in [
        (Some(0), &b""[..]),
        (Some(113), &b"permission denied"[..]),
        (None, &b""[..]),
        (Some(113), &b"\xff"[..]),
        (
            Some(113),
            &b"Could not find service \"other\" in domain for user gui: 501"[..],
        ),
    ] {
        assert!(registration::recovery::require_unloaded_output(501, code, text).is_err());
    }
    assert!(registration::recovery::require_unloaded_output(0, Some(113), expected).is_err());
    for loaded in [
        b"pid = 0".as_slice(),
        b"pid = 42".as_slice(),
        b"state = not running".as_slice(),
        b"different registration".as_slice(),
    ] {
        assert!(registration::recovery::require_unloaded_output(501, Some(0), loaded).is_err());
    }
    assert!(
        registration::recovery::require_unloaded_output(501, Some(113), &vec![b' '; 4097]).is_err()
    );
}
struct LoadedRunner;
#[async_trait]
impl ServiceRunner for LoadedRunner {
    async fn run(&self, _: &[String]) -> Result<()> {
        panic!("must never control a loaded legacy job")
    }
    async fn require_unloaded(&self) -> Result<()> {
        Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            registration::recovery::EXTERNAL_UNLOAD_REQUIRED,
        ))
    }
}
#[tokio::test]
async fn desktop_service_recovery_loaded_or_reloaded_registration_never_controls_job() {
    let f = Fixture::new().await;
    let before = tree(&f.home);
    let (value, plan) = omm::application::lifecycle::plan_at(
        &f.paths,
        Some(&f.bundle),
        "switch",
        &f.home,
        &f.runtime,
        &LoadedRunner,
    )
    .await
    .unwrap();
    assert!(plan.is_none());
    assert_eq!(value["canApply"], false);
    assert!(value["reasons"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("external_unload_required")));
    assert!(value["guidance"]
        .as_str()
        .unwrap()
        .contains("Do not use service uninstall"));
    assert_eq!(before, tree(&f.home));
    let plan = f.plan().unwrap();
    assert!(recovery::adopt(
        &f.paths,
        &f.home,
        &f.runtime,
        &f.bundle,
        &plan,
        &LoadedRunner
    )
    .await
    .is_err());
    assert!(profile::read(&f.paths).unwrap().is_none());
    assert_eq!(fs::read(f.plist()).unwrap(), before[&f.plist()]);
}
struct FakeNotifier;
#[async_trait]
impl omm::core::notifications::NotificationAdapter for FakeNotifier {
    async fn send(&self, _: omm::core::notifications::NotificationEvent) -> Result<()> {
        panic!("no notification may be sent by this fixture")
    }
}
#[tokio::test]
async fn desktop_service_recovery_failed_start_does_not_publish_owner_generation() {
    for adopted in [false, true] {
        let f = Fixture::new().await;
        if adopted {
            recovery::adopt(
                &f.paths,
                &f.home,
                &f.runtime,
                &f.bundle,
                &f.plan().unwrap(),
                &f.runner,
            )
            .await
            .unwrap();
        }
        let owner = f.paths.state.join("desktop-owner.json");
        let previous = fs::read(&owner).ok();
        write_private_replace(
            &f.paths.state.join("state.sqlite"),
            b"not a sqlite database",
        )
        .unwrap();
        let result = omm::service::daemon::run_at_with_notifier(
            f.paths.clone(),
            f.runtime.clone(),
            std::sync::Arc::new(FakeNotifier),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(previous, fs::read(&owner).ok());
    }
}

const LEGACY_ASR_DDL: &str = include_str!("fixtures/desktop-service-recovery-asr-51319b7.sql");

fn recovery_bytes(f: &Fixture) -> BTreeMap<String, Option<String>> {
    let mut hashes = BTreeMap::new();
    for (name, path) in [
        ("config", f.paths.config.join("config.json")),
        ("database", f.paths.state.join("state.sqlite")),
        ("wal", f.paths.state.join("state.sqlite-wal")),
        ("shm", f.paths.state.join("state.sqlite-shm")),
        ("journal", f.paths.state.join("state.sqlite-journal")),
    ] {
        hashes.insert(
            name.to_owned(),
            path.exists().then(|| hash_file(&path).unwrap()),
        );
    }
    hashes
}

#[tokio::test]
async fn desktop_service_recovery_historical_asr_optional_prefixes_adopt_without_writes_or_start() {
    use omm::application::lifecycle;
    // The historical execute_batch has no transaction around the two CREATEs:
    // absent, first-only (interrupted), and both are the reachable schema prefixes.
    for count in 0..=2 {
        let f = Fixture::new().await;
        let c = rusqlite::Connection::open(f.paths.state.join("state.sqlite")).unwrap();
        let ddl = LEGACY_ASR_DDL
            .split_inclusive(';')
            .take(count)
            .collect::<String>();
        c.execute_batch(&ddl).unwrap();
        if count >= 1 {
            c.execute(
                "INSERT INTO asr_measurements VALUES('synthetic-task',1,1,'{}')",
                [],
            )
            .unwrap();
        }
        if count == 2 {
            c.execute("INSERT INTO asr_measurement_totals VALUES('synthetic-task','{\"observations\":1}')", []).unwrap();
        }
        // No historical explicit indexes/triggers: only SQLite primary-key autoindexes.
        let objects: Vec<(String, String, bool)> = c.prepare("SELECT type,name,sql IS NULL FROM sqlite_master WHERE tbl_name IN ('asr_measurements','asr_measurement_totals') ORDER BY name")
            .unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(|r| r.unwrap()).collect();
        let mut expected = vec![];
        if count == 2 {
            expected.push(("table".into(), "asr_measurement_totals".into(), false));
        }
        if count >= 1 {
            expected.push(("table".into(), "asr_measurements".into(), false));
        }
        if count == 2 {
            expected.push((
                "index".into(),
                "sqlite_autoindex_asr_measurement_totals_1".into(),
                true,
            ));
        }
        if count >= 1 {
            expected.push((
                "index".into(),
                "sqlite_autoindex_asr_measurements_1".into(),
                true,
            ));
        }
        assert_eq!(objects, expected);
        drop(c);
        let before = recovery_bytes(&f);
        f.plan().expect("historical ASR schema must be supported");
        assert_eq!(before, recovery_bytes(&f));
        let (value, plan) = lifecycle::plan_at(
            &f.paths,
            Some(&f.bundle),
            "switch",
            &f.home,
            &f.runtime,
            &f.runner,
        )
        .await
        .unwrap();
        assert_eq!(
            value["state"], "stopped",
            "optional prefix {count}: {value}"
        );
        assert_eq!(value["canApply"], true, "optional prefix {count}: {value}");
        assert_eq!(before, recovery_bytes(&f));
        let adopted = lifecycle::apply_at(
            &f.paths,
            &f.bundle,
            plan.unwrap(),
            &f.home,
            &f.runtime,
            &f.runner,
        )
        .await
        .unwrap();
        assert_eq!(adopted["state"], "stopped");
        assert!(profile::read(&f.paths).unwrap().is_some());
        assert!(f.runner.calls.lock().unwrap().is_empty());
        assert!(omm::core::runtime::verify_active(&f.runtime.path, &f.paths.state).is_err());
        assert_eq!(before, recovery_bytes(&f));
        println!(
            "historical optional prefix {count}: preserved config/database/sidecars {}",
            serde_json::to_string(&before).unwrap()
        );
    }
}

#[tokio::test]
async fn desktop_service_recovery_historical_asr_rejects_deformation_and_unreachable_suffix() {
    for sql in [
        "ALTER TABLE asr_measurements ADD COLUMN unexpected TEXT",
        "DROP TABLE asr_measurement_totals; CREATE TABLE asr_measurement_totals(task TEXT,document TEXT NOT NULL)",
        "CREATE INDEX extra_measurement_index ON asr_measurements(ordinal)",
        "CREATE INDEX extra_totals_index ON asr_measurement_totals(document)",
        "CREATE TRIGGER extra_measurement_trigger AFTER INSERT ON asr_measurements BEGIN SELECT 1; END",
        "DROP TABLE asr_measurements; CREATE VIEW asr_measurements AS SELECT 1 AS task",
        // Second-only is not produced by the historical ordered CREATEs.
        "DROP TABLE asr_measurements",
        "CREATE TABLE unknown_observer(value TEXT)",
        "PRAGMA user_version=100",
        "DROP TABLE settings",
    ] {
        let f = Fixture::new().await;
        let c = rusqlite::Connection::open(f.paths.state.join("state.sqlite")).unwrap();
        c.execute_batch(LEGACY_ASR_DDL).unwrap();
        c.execute_batch(sql).unwrap();
        drop(c);
        let before = tree(&f.home);
        let error = match f.plan() {
            Err(error) => error,
            Ok(_) => panic!("must reject {sql}"),
        };
        assert!(error.message.contains("schema is unsupported"), "{sql}: {error}");
        assert_eq!(before, tree(&f.home));
        assert!(f.runner.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn desktop_service_recovery_literal_sqlite_prefix_rejects_user_objects() {
    use omm::application::lifecycle;
    let mut admitted = Vec::new();
    for (kind, sql) in [
        ("index", "CREATE INDEX sqliteXobserver ON asr_measurements(ordinal)"),
        ("mixed-case index", "CREATE INDEX SQLITExobserver ON asr_measurement_totals(document)"),
        ("trigger", "CREATE TRIGGER sqliteXobserver AFTER INSERT ON asr_measurements BEGIN DELETE FROM asr_measurement_totals; END"),
        ("table", "CREATE TABLE sqliteXobserver(value TEXT)"),
        ("view", "CREATE VIEW sqliteXobserver AS SELECT task FROM asr_measurements"),
    ] {
        for modern in [false, true] {
            let f = Fixture::new().await;
            let db = f.paths.state.join("state.sqlite");
            rusqlite::Connection::open(&db)
                .unwrap()
                .execute_batch(LEGACY_ASR_DDL)
                .unwrap();
            if modern {
                let plan = f.plan().unwrap();
                recovery::adopt(&f.paths, &f.home, &f.runtime, &f.bundle, &plan, &f.runner)
                    .await
                    .unwrap();
                drop(Store::open(&f.paths).unwrap());
                let (value, plan) = lifecycle::plan_at(
                    &f.paths,
                    Some(&f.bundle),
                    "start",
                    &f.home,
                    &f.runtime,
                    &f.runner,
                )
                .await
                .unwrap();
                assert_eq!(value["canApply"], true, "unmodified modern schema: {value}");
                assert!(plan.is_some());
            }
            let c = rusqlite::Connection::open(&db).unwrap();
            // These are legal user names, not the reserved literal sqlite_ prefix.
            // Never insert through the synthetic destructive trigger.
            c.execute_batch(sql).unwrap();
            let objects: Vec<(String, String)> = c
                .prepare("SELECT type,name FROM sqlite_master WHERE name GLOB '*observer'")
                .unwrap()
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .collect();
            assert_eq!(objects.len(), 1, "{sql}");
            drop(c);
            let before = tree(f._temp.path());
            let label = format!("{kind}, modern={modern}");
            if !modern {
                match f.plan() {
                    Err(error) => assert!(
                        error.message.contains("schema is unsupported"),
                        "{label}: {error}"
                    ),
                    Ok(_) => admitted.push(format!("raw {label}")),
                }
            }
            let (value, plan) = lifecycle::plan_at(
                &f.paths,
                Some(&f.bundle),
                if modern { "start" } else { "switch" },
                &f.home,
                &f.runtime,
                &f.runner,
            )
            .await
            .unwrap();
            if value["state"] != "incompatible"
                || value["canApply"] != false
                || plan.is_some()
                || !value["reasons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r == "incompatible_schema")
            {
                admitted.push(format!("lifecycle {label}: {value}"));
            }
            assert_eq!(before, tree(f._temp.path()), "{label}: plan changed fixture bytes");
            assert!(f.runner.calls.lock().unwrap().is_empty(), "{label}");
            println!("checked {label}: {objects:?}; fixture unchanged; runner calls=0");
        }
    }
    assert!(
        admitted.is_empty(),
        "unknown user objects were admitted: {admitted:#?}"
    );
}

// Exercise the normal Store measurement writer with a live synthetic task lease,
// then settle it through Store::fail. No worker/model, audio file or helper is used.
fn record_modern_asr_measurement(f: &Fixture) -> uuid::Uuid {
    let now = chrono::Utc::now().timestamp_millis();
    let task = Task {
        id: uuid::Uuid::new_v4(),
        session_id: uuid::Uuid::new_v4(),
        attempt: 1,
        stage: Stage::Transcribe,
        status: TaskStatus::Transcribing,
        input_hash: "synthetic".into(),
        config_hash: "synthetic".into(),
        summary_metadata: None,
        transcript_source: None,
        error: None,
    };
    let lease = Lease {
        token: uuid::Uuid::new_v4(),
        expires_at_ms: now + 60000,
    };
    let context = TaskContext {
        session_id: task.session_id,
        task_id: task.id,
        attempt: 1,
        lease: lease.clone(),
        meeting_dir: f.paths.data.clone(),
        work_dir: f.paths.cache.clone(),
        config: ConfigSnapshot::default(),
    };
    let mut store = Store::open(&f.paths).unwrap();
    let c = rusqlite::Connection::open(f.paths.state.join("state.sqlite")).unwrap();
    c.execute(
        "INSERT INTO meetings VALUES(?1,'{}')",
        [task.session_id.to_string()],
    )
    .unwrap();
    c.execute("INSERT INTO tasks(id,session,attempt,input_hash,config_hash,document,stage,status,lease,expires,directory,config,segments) VALUES(?1,?2,1,'synthetic','synthetic',?3,'transcribe','transcribing',?4,?5,?6,?7,'[]')", rusqlite::params![task.id.to_string(), task.session_id.to_string(), serde_json::to_string(&task).unwrap(), lease.token.to_string(), lease.expires_at_ms, context.meeting_dir.to_str(), serde_json::to_string(&context.config).unwrap()]).unwrap();
    drop(c);
    let event: AsrWorkerEvent = serde_json::from_value(serde_json::json!({
        "type":"chunk_progress", "schema_version":1, "task_id":task.id, "attempt":1,
        "source":"mic", "completed_chunks":1, "vad_spans":1, "speech_samples":3200, "processing_ms":40,
        "chunk_timing":{"phase":"native_decode_total", "input_index":0, "window_index":0,
            "core_start_sample":1600, "decode_start_sample":0, "core_samples":3200, "decode_samples":8000,
            "cache_hit":false, "cache_lookup_us":3, "native_decode_wall_us":20000,
            "process_cpu_us":35000, "checkpoint_write_us":4, "token_count":2, "output_chars":7}
    })).unwrap();
    store.record_asr_measurement_best_effort(&context, &event);
    let measurements = store.asr_measurements(task.id).unwrap();
    assert_eq!(measurements["totals"]["observations"], 1);
    assert_eq!(measurements["samples"].as_array().unwrap().len(), 1);
    let claim = ClaimedTask {
        stage: Stage::Transcribe,
        task,
        lease,
        context,
        segments: vec![],
    };
    store
        .fail(
            &claim,
            &OmmError::unavailable("synthetic task settled without worker"),
            now,
        )
        .unwrap();
    claim.task.id
}

use clap::Parser;
use omm::{
    cli::{self, Cli, Commands},
    core::{config::AppConfig, contracts::*, paths::*, runtime::*},
    service::daemon,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};
use tempfile::TempDir;

struct Fixture {
    dir: TempDir,
    paths: Paths,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("omm-r-")
            .tempdir_in("/private/tmp")
            .unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let paths = Paths::resolve(&dir.path().join("home"), &BTreeMap::new()).unwrap();
        Self { dir, paths }
    }
    fn location(&self) -> RuntimeLocation {
        RuntimeLocation::resolve(&BTreeMap::new(), || Ok(self.dir.path().to_owned())).unwrap()
    }
    fn initialize(&self) {
        self.paths.create_private_dirs().unwrap();
        write_private_new(
            &self.paths.config.join("config.json"),
            &serde_json::to_vec(&AppConfig::default()).unwrap(),
        )
        .unwrap();
    }
    fn command(&self, action: &str, args: &[&str], explicit: Option<&std::path::Path>) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "fixture_runtime_process", "--nocapture"])
            .env_clear()
            .env("HOME", self.dir.path().join("home"))
            .env("OMM_FIXTURE_ACTION", action)
            .env("OMM_FIXTURE_TEMP", self.dir.path())
            .env("OMM_FIXTURE_ARGS", serde_json::to_string(args).unwrap());
        if let Some(path) = explicit {
            command.env("XDG_RUNTIME_DIR", path);
        }
        command
    }
    fn cli(&self, args: &[&str], explicit: Option<&std::path::Path>) -> Output {
        self.command("cli", args, explicit).output().unwrap()
    }
    fn start(&self, explicit: Option<&std::path::Path>) -> ServiceChild {
        ServiceChild(
            self.command("service", &[], explicit)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
    fn await_ready(&self, explicit: Option<&std::path::Path>) {
        let start = Instant::now();
        loop {
            if self
                .cli(&["service", "status", "--json"], explicit)
                .status
                .success()
            {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}
struct ServiceChild(Child);
impl Drop for ServiceChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn json(output: &Output) -> serde_json::Value {
    let text = String::from_utf8(output.stdout.clone()).unwrap();
    serde_json::from_str(
        text.lines()
            .find(|s| s.starts_with('{'))
            .expect("JSON response"),
    )
    .unwrap()
}

#[test]
fn fixture_runtime_process() {
    let Ok(action) = std::env::var("OMM_FIXTURE_ACTION") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("OMM_FIXTURE_TEMP").unwrap());
    let location = RuntimeLocation::resolve(&std::env::vars().collect(), || Ok(root));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let code = if action == "service" {
        match location.and_then(|l| Ok((Paths::from_env()?, l))) {
            Ok((p, l)) => runtime
                .block_on(daemon::run_at(p, l))
                .map(|_| 0)
                .unwrap_or_else(|e| e.exit_code()),
            Err(e) => e.exit_code(),
        }
    } else {
        let args: Vec<String> =
            serde_json::from_str(&std::env::var("OMM_FIXTURE_ARGS").unwrap()).unwrap();
        let parsed = Cli::parse_from(std::iter::once("omm".to_owned()).chain(args));
        runtime.block_on(cli::run_with_runtime(parsed, Some(location)))
    };
    std::process::exit(code);
}
#[tokio::test]
async fn readonly_no_env_paths_report_replacement_without_creating_resources() {
    let f = Fixture::new();
    let location = f.location();
    let data = cli::execute_at(&Commands::Paths, &f.paths, true, &Ok(location.clone()))
        .await
        .unwrap();
    assert_eq!(data["runtimeSource"], "macos_replacement");
    assert_eq!(data["runtimeLifecycle"], "best_effort");
    let doctor = cli::execute_at(&Commands::Doctor, &f.paths, true, &Ok(location.clone()))
        .await
        .unwrap();
    assert_eq!(doctor["runtimeSource"], "macos_replacement");
    assert_eq!(doctor["runtimeLifecycle"], "best_effort");
    assert_eq!(doctor["runtimeReady"], false);
    assert!(!location.path.exists());
    assert!(!f.paths.config.exists());
}
#[test]
fn selection_prioritizes_valid_xdg_and_refuses_path_attacks_instead_of_fallback() {
    let f = Fixture::new();
    let root = f.dir.path();
    let mut env = BTreeMap::new();
    env.insert("XDG_RUNTIME_DIR".into(), root.to_str().unwrap().into());
    let explicit = RuntimeLocation::resolve(&env, || panic!("explicit must win")).unwrap();
    assert_eq!(explicit.source, "explicit_xdg");
    assert!(explicit.warnings.is_empty());
    env.insert(
        "XDG_RUNTIME_DIR".into(),
        root.join("missing").to_str().unwrap().into(),
    );
    assert!(RuntimeLocation::resolve(&env, || Ok(root.into()))
        .unwrap()
        .warnings[0]
        .contains("does not exist"));
    env.insert("XDG_RUNTIME_DIR".into(), "relative".into());
    assert_eq!(
        RuntimeLocation::resolve(&env, || panic!("attack must not fallback"))
            .unwrap_err()
            .code,
        ErrorCode::Permission
    );
    let bad = root.join("wide");
    fs::create_dir(&bad).unwrap();
    fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();
    env.insert("XDG_RUNTIME_DIR".into(), bad.to_str().unwrap().into());
    assert_eq!(
        RuntimeLocation::resolve(&env, || panic!())
            .unwrap_err()
            .code,
        ErrorCode::Permission
    );
    let link = root.join("link");
    symlink(root.join("missing-target"), &link).unwrap();
    env.insert("XDG_RUNTIME_DIR".into(), link.to_str().unwrap().into());
    assert_eq!(
        RuntimeLocation::resolve(&env, || panic!())
            .unwrap_err()
            .code,
        ErrorCode::Permission
    );
    env.insert(
        "XDG_RUNTIME_DIR".into(),
        link.join("absent").to_str().unwrap().into(),
    );
    assert_eq!(
        RuntimeLocation::resolve(&env, || panic!())
            .unwrap_err()
            .code,
        ErrorCode::Permission
    );
    env.clear();
    env.insert("TMPDIR".into(), link.to_str().unwrap().into());
    env.insert("OMM_RUNTIME_DIR".into(), link.to_str().unwrap().into());
    assert_eq!(
        RuntimeLocation::resolve(&env, || Ok(root.into()))
            .unwrap()
            .path,
        f.location().path
    );
    let replacement = f.location();
    fs::create_dir(&replacement.path).unwrap();
    fs::set_permissions(&replacement.path, fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(
        RuntimeLocation::resolve(&BTreeMap::new(), || Ok(root.into()))
            .unwrap_err()
            .code,
        ErrorCode::Permission
    );
}
#[test]
fn no_env_service_and_multiple_cli_clients_share_location_warn_and_clean_normally() {
    let f = Fixture::new();
    f.initialize();
    let mut service = f.start(None);
    f.await_ready(None);
    let paths = f.cli(&["paths", "--json"], None);
    let data = json(&paths);
    assert_eq!(data["data"]["runtimeSource"], "macos_replacement");
    assert_eq!(data["data"]["runtimeLifecycle"], "best_effort");
    let stderr = String::from_utf8(paths.stderr).unwrap();
    let warnings: Vec<String> = data["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().into())
        .collect();
    assert_eq!(warnings, f.location().warnings);
    assert_eq!(
        stderr
            .lines()
            .map(|s| s.strip_prefix("warning: ").unwrap().to_owned())
            .collect::<Vec<_>>(),
        warnings
    );
    let statuses = std::thread::scope(|scope| {
        (0..4)
            .map(|_| scope.spawn(|| f.cli(&["service", "status", "--json"], None)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    for status in statuses {
        assert!(status.status.success());
        assert_eq!(
            json(&status)["data"]["runtime"]["path"],
            f.location().path.to_str().unwrap()
        );
    }
    let before = fs::read(f.location().path.join("owner.json")).unwrap();
    let mut duplicate = f.start(None);
    assert_eq!(duplicate.0.wait().unwrap().code(), Some(6));
    assert_eq!(
        fs::read(f.location().path.join("owner.json")).unwrap(),
        before
    );
    assert!(f.cli(&["service", "stop", "--json"], None).status.success());
    assert_eq!(service.0.wait().unwrap().code(), Some(0));
    assert!(!f.location().path.join("owner.json").exists());
    assert!(!f.location().path.join("service.sock").exists());
    assert!(f.paths.state.join("state.sqlite").is_file());
}
#[test]
fn sigkill_recovers_only_identified_dead_owner_and_rejects_same_state_other_runtime() {
    let f = Fixture::new();
    f.initialize();
    let mut service = f.start(None);
    f.await_ready(None);
    let old = read_owner(&f.location().path, &f.paths.state).unwrap();
    service.0.kill().unwrap();
    service.0.wait().unwrap();
    assert!(f.location().path.join("owner.json").exists());
    let mut restarted = f.start(None);
    f.await_ready(None);
    assert_ne!(
        read_owner(&f.location().path, &f.paths.state)
            .unwrap()
            .instance_id,
        old.instance_id
    );
    let other = Fixture::new();
    let mut split = f.start(Some(other.dir.path()));
    assert_eq!(split.0.wait().unwrap().code(), Some(6));
    assert!(f
        .cli(&["service", "status", "--json"], None)
        .status
        .success());
    assert!(!other.dir.path().join("omm/service.sock").exists());
    assert!(f.cli(&["service", "stop", "--json"], None).status.success());
    assert_eq!(restarted.0.wait().unwrap().code(), Some(0));
}
#[test]
fn explicitly_isolated_state_and_runtime_do_not_delete_each_other() {
    let a = Fixture::new();
    let b = Fixture::new();
    a.initialize();
    b.initialize();
    let mut one = a.start(None);
    let mut two = b.start(Some(b.dir.path()));
    a.await_ready(None);
    b.await_ready(Some(b.dir.path()));
    assert!(a.cli(&["service", "stop", "--json"], None).status.success());
    assert_eq!(one.0.wait().unwrap().code(), Some(0));
    assert!(b
        .cli(&["service", "status", "--json"], Some(b.dir.path()))
        .status
        .success());
    assert!(b
        .cli(&["service", "stop", "--json"], Some(b.dir.path()))
        .status
        .success());
    assert_eq!(two.0.wait().unwrap().code(), Some(0));
}
#[test]
fn guard_refuses_unidentified_resources_and_never_cleans_replaced_instance_marker() {
    let f = Fixture::new();
    f.paths.create_private_dirs().unwrap();
    let location = f.location();
    let guard = RuntimeGuard::acquire(&location, &f.paths.state).unwrap();
    let mut changed = guard.owner.clone();
    changed.instance_id = uuid::Uuid::new_v4();
    write_private_replace(
        &location.path.join("owner.json"),
        &serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    drop(guard);
    assert_eq!(read_owner(&location.path, &f.paths.state).unwrap(), changed);
    assert!(RuntimeGuard::acquire(&location, &f.paths.state).is_err());
    let mut foreign = changed.clone();
    foreign.uid = foreign.uid.wrapping_add(1);
    write_private_replace(
        &location.path.join("owner.json"),
        &serde_json::to_vec(&foreign).unwrap(),
    )
    .unwrap();
    assert!(RuntimeGuard::acquire(&location, &f.paths.state).is_err());
    assert!(location.path.join("owner.json").exists());
    fs::remove_file(location.path.join("owner.json")).unwrap();
    let socket =
        std::os::unix::net::UnixListener::bind(location.path.join("service.sock")).unwrap();
    fs::set_permissions(
        location.path.join("service.sock"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(RuntimeGuard::acquire(&location, &f.paths.state).is_err());
    assert!(location.path.join("service.sock").exists());
    drop(socket);
}
#[test]
fn runtime_attack_errors_do_not_fallback_and_short_socket_limit_is_diagnostic() {
    let f = Fixture::new();
    let unsafe_root = f.dir.path().join("wide-SECRET_FIXTURE");
    fs::create_dir(&unsafe_root).unwrap();
    fs::set_permissions(&unsafe_root, fs::Permissions::from_mode(0o755)).unwrap();
    let output = f.cli(&["service", "status", "--json"], Some(&unsafe_root));
    assert_eq!(output.status.code(), Some(3));
    let response = json(&output);
    assert_eq!(response["error"]["code"], "permission");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("SECRET_FIXTURE"));
    assert!(!stderr.contains("replacement selected"));
    assert_eq!(
        stderr
            .lines()
            .map(|s| s.strip_prefix("warning: ").unwrap())
            .collect::<Vec<_>>(),
        response["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
    );
    let long = f.dir.path().join("l".repeat(90));
    private_dir(&long).unwrap();
    let env = BTreeMap::from([("XDG_RUNTIME_DIR".into(), long.to_str().unwrap().into())]);
    let error = RuntimeLocation::resolve(&env, || panic!("length must not fallback")).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert!(error.message.contains("shorter valid explicit"));
    assert!(!long.join("omm").exists());
}

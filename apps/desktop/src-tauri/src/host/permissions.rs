//! Actual typed Host dispatcher and Unix service transport; only platform compatibility is injected.
const PERMISSION_PEER: &str = r#"
import json,os,socket,sys
path,instance,marker=sys.argv[1:]
s=socket.socket(socket.AF_UNIX);s.bind(path);os.chmod(path,0o600);s.listen();s.settimeout(10)
try:
 while True:
  c,_=s.accept();data=b''
  while True:
   b=c.recv(65536)
   if not b:break
   data+=b
  r=json.loads(data);a=r['action'];v={'instance_id':instance,'captureIPCVersion':1,'pid':os.getpid(),'recording':False,'sysReady':False,'micReady':False}
  if a in ('permissions_request','permissions_cancel'):
   with open(marker,'a') as f:f.write(a+'\n')
   v=dict(instance_id=instance,request_id=r['request_id'],microphone='denied',systemAudio='unknown',outcome='cancelled' if a=='permissions_cancel' else 'completed')
  if a=='quit':v['status']='bye'
  c.sendall(json.dumps(v).encode());c.close()
  if a=='quit':break
finally:
 s.close();os.unlink(path)
"#;

fn permission_fixture_python(path: &std::ffi::OsStr) -> std::io::Result<std::path::PathBuf> {
    // Resolve before env_clear; never search the current directory or inherit Python context.
    for directory in std::env::split_paths(path).filter(|directory| directory.is_absolute()) {
        let Ok(candidate) = directory.join("python3").canonicalize() else {
            continue;
        };
        let metadata = candidate.metadata()?;
        if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 {
            return Ok(candidate);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "native permission fixture requires executable python3 in an absolute PATH directory",
    ))
}

#[test]
fn permission_fixture_python_missing_is_an_explicit_error() {
    let temp = tempfile::tempdir().unwrap();
    let path =
        std::env::join_paths([temp.path(), std::path::Path::new("relative-unused")]).unwrap();
    let error = permission_fixture_python(&path).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(error.to_string().contains("python3"));
}

#[tokio::test]
async fn permission_fixture_python_uses_alternate_absolute_path_entries() {
    let python = permission_fixture_python(&std::env::var_os("PATH").unwrap_or_default())
        .expect("native permission fixture requires python3");
    let temp = tempfile::tempdir().unwrap();
    for name in ["first", "second"] {
        let bin = temp.path().join(name);
        std::fs::create_dir(&bin).unwrap();
        let alias = bin.join("python3");
        std::os::unix::fs::symlink(&python, &alias).unwrap();
        let path = std::env::join_paths([std::path::Path::new("relative-unused"), &bin]).unwrap();
        let resolved = permission_fixture_python(&path).unwrap();
        assert_eq!(resolved, alias.canonicalize().unwrap());
        host_core_service_permissions_fixture(&resolved).await;
    }
}

#[tokio::test]
async fn host_core_service_permissions_reject_stale_instance_before_any_action() {
    let python = permission_fixture_python(&std::env::var_os("PATH").unwrap_or_default())
        .expect("native permission fixture requires python3");
    host_core_service_permissions_fixture(&python).await;
}

async fn host_core_service_permissions_fixture(python: &std::path::Path) {
    use omm::core::{
        config::AppConfig,
        notifications::{NotificationAdapter, NotificationEvent},
        store::Store,
    };
    struct NoNotifications;
    #[async_trait::async_trait]
    impl NotificationAdapter for NoNotifications {
        async fn send(&self, _: NotificationEvent) -> Result<()> {
            panic!("no real notifications in fixture")
        }
    }
    let temp = tempfile::Builder::new()
        .prefix("omm-hgen-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("h"), &BTreeMap::new()).unwrap();
    paths.create_private_dirs().unwrap();
    let mut cfg = AppConfig::default();
    cfg.notifications.enabled = false;
    cfg.auto_record = false;
    cfg.save(&paths).unwrap();
    let rt = root.join("r");
    private_dir(&rt).unwrap();
    let runtime = RuntimeLocation::resolve(
        &BTreeMap::from([("XDG_RUNTIME_DIR".into(), rt.to_str().unwrap().into())]),
        || unreachable!(),
    )
    .unwrap();
    private_dir(&runtime.path).unwrap();
    private_dir(&runtime.path.join("capture")).unwrap();
    let previous = RuntimeGuard::acquire(&runtime, &paths.state).unwrap();
    profile::takeover(&paths, &previous).unwrap();
    let previous_generation = previous.owner.instance_id;
    drop(previous);
    let instance = Uuid::new_v4();
    Store::open(&paths)
        .unwrap()
        .set_state("capture_instance", &instance)
        .unwrap();
    let marker = root.join("permission-actions");
    let child = std::process::Command::new(python)
        .env_clear()
        .env("HOME", root.join("h"))
        .args(["-B", "-c", PERMISSION_PEER])
        .arg(runtime.path.join("capture/audio_daemon.sock"))
        .arg(instance.to_string())
        .arg(&marker)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !runtime.path.join("capture/audio_daemon.sock").exists() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let service = tokio::spawn(daemon::run_at_with_notifier(
        paths.clone(),
        runtime.clone(),
        std::sync::Arc::new(NoNotifications),
    ));
    let owner = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if runtime.path.join("service.sock").exists() {
                if let Ok(Some(p)) = profile::read(&paths) {
                    break p;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let mut stale = vec![];
    for command in ["permissions_request", "permissions_cancel"] {
        // Raw wire deliberately bypasses the client fence: the real service must defend itself.
        let mut stream = tokio::net::UnixStream::connect(runtime.path.join("service.sock"))
            .await
            .unwrap();
        stream
            .write_all(
                &serde_json::to_vec(&daemon::Request {
                    schema_version: 1,
                    command: command.into(),
                    title: None,
                    session_id: Some(Uuid::new_v4()),
                    instance_id: Some(previous_generation),
                    desktop_request: None,
                    desktop_capability: None,
                })
                .unwrap(),
            )
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
        let mut bytes = vec![];
        stream.read_to_end(&mut bytes).await.unwrap();
        stale.push(serde_json::from_slice::<daemon::Reply>(&bytes).unwrap());
    }
    let actions_before = std::fs::read_to_string(&marker).unwrap_or_default();
    let mut host = Host::new(paths.clone());
    host.test_runtime = Some(runtime.clone());
    host.permission_compatibility = Some(profile::compatible(&owner, "service"));
    let envelope = json!({"schemaVersion":1,"requestId":Uuid::new_v4(),"payload":{"action":"request","operationId":Uuid::new_v4()}});
    let host_result = host.dispatch("permissions_request", &envelope).await;
    // Correct supplied generation and existing CLI None are both preserved, on the actual service loop.
    let mut accepted = vec![];
    for generation in [Some(owner.generation), None] {
        for command in ["permissions_request", "permissions_cancel"] {
            accepted.push(
                daemon::request_message_at(
                    &paths,
                    &runtime,
                    daemon::Request {
                        schema_version: 1,
                        command: command.into(),
                        title: None,
                        session_id: Some(Uuid::new_v4()),
                        instance_id: generation,
                        desktop_request: None,
                        desktop_capability: None,
                    },
                )
                .await,
            );
        }
    }
    let stopped = daemon::request_at(&paths, &runtime, "shutdown", None).await;
    let service_result = service.await.unwrap();
    // Actual finite fake Child wait consumes both pipes; no Drop/timeout signals or proxy PID claims.
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    assert!(stopped.is_ok() && service_result.is_ok());
    assert!(
        actions_before.is_empty(),
        "stale generation reached fake permission API: {actions_before}"
    );
    for reply in stale {
        assert!(!reply.ok);
        assert_eq!(reply.error.unwrap().code, ErrorCode::Conflict);
    }
    assert_eq!(host_result.unwrap()["microphone"], "denied");
    assert!(accepted.iter().all(Result::is_ok));
    let actions = std::fs::read_to_string(marker).unwrap();
    assert_eq!(actions.lines().count(), 5);
    assert!(std::fs::read_dir(paths.data.join("meetings"))
        .unwrap()
        .next()
        .is_none());
}

use super::*;
use omm::core::{paths::*, runtime::*};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
    sync::Notify,
};
#[tokio::test]
async fn host_permission_request_cancel_share_one_typed_command_and_owned_connection() {
    let temp = tempfile::Builder::new()
        .prefix("omm-host-p-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let root = temp.path().canonicalize().unwrap();
    let paths = Paths::resolve(&root.join("h"), &BTreeMap::new()).unwrap();
    paths.create_private_dirs().unwrap();
    let rt = root.join("r");
    private_dir(&rt).unwrap();
    let runtime = RuntimeLocation::resolve(
        &BTreeMap::from([("XDG_RUNTIME_DIR".into(), rt.to_str().unwrap().into())]),
        || unreachable!(),
    )
    .unwrap();
    let guard = RuntimeGuard::acquire(&runtime, &paths.state).unwrap();
    profile::takeover(&paths, &guard).unwrap();
    let socket = runtime.path.join("service.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut host = Host::new(paths);
    host.test_runtime = Some(runtime);
    host.permission_compatibility = Some(
        json!({"admissionMode":"service","writesAllowed":true,"serviceGeneration":guard.owner.instance_id}),
    );
    let ui = Uuid::new_v4();
    let envelope = |action: &str| json!({"schemaVersion":1,"requestId":Uuid::new_v4().to_string(),"payload":{"action":action,"operationId":ui}});
    let request = envelope("request");
    let cancel = envelope("cancel");
    let arrived = Notify::new();
    let backend = async {
        let (mut original, _) = listener.accept().await.unwrap();
        let mut bytes = vec![];
        original.read_to_end(&mut bytes).await.unwrap();
        let original_req: daemon::Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(original_req.command, "permissions_request");
        assert_ne!(original_req.session_id, Some(ui));
        arrived.notify_one();
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = vec![];
        stream.read_to_end(&mut bytes).await.unwrap();
        let cancel: daemon::Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(cancel.command, "permissions_cancel");
        assert_eq!(cancel.session_id, original_req.session_id);
        stream
            .write_all(
                &serde_json::to_vec(&daemon::Reply {
                    ok: true,
                    data: Some(json!({"outcome":"cancelled"})),
                    error: None,
                    service_instance: Some(guard.owner.instance_id),
                })
                .unwrap(),
            )
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    };
    let cancellation = async {
        arrived.notified().await;
        host.dispatch("permissions_request", &cancel).await
    };
    let (result, cancelled, ()) = tokio::join!(
        host.dispatch("permissions_request", &request),
        cancellation,
        backend
    );
    assert_eq!(result.unwrap()["outcome"], "cancelled");
    assert_eq!(cancelled.unwrap()["outcome"], "cancelled");
    assert!(host.dispatch("permissions_request", &cancel).await.is_err());
    let mut bad = request.clone();
    bad["payload"]["window"] = json!("main");
    assert!(host.dispatch("permissions_request", &bad).await.is_err());
    assert!(std::fs::read_dir(&host.paths.data)
        .unwrap()
        .next()
        .is_none());
}

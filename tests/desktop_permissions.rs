use omm::{
    application::{permissions::Requests, profile},
    core::{paths::*, runtime::*},
    service::daemon::{Reply, Request},
};
use serde_json::{json, Value};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
};
use uuid::Uuid;
struct Fixture {
    _temp: tempfile::TempDir,
    paths: Paths,
    runtime: RuntimeLocation,
    _guard: RuntimeGuard,
    listener: UnixListener,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("omm-dperm-")
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
        Self {
            _temp: temp,
            paths,
            runtime,
            _guard: guard,
            listener,
        }
    }
}
fn payload(action: &str, id: Uuid) -> Value {
    json!({"action":action,"operationId":id})
}
async fn read(stream: &mut tokio::net::UnixStream) -> Request {
    let mut bytes = vec![];
    stream.read_to_end(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
async fn reply(stream: &mut tokio::net::UnixStream, owner: Uuid, data: Value) {
    stream
        .write_all(
            &serde_json::to_vec(&Reply {
                ok: true,
                data: Some(data),
                error: None,
                service_instance: Some(owner),
            })
            .unwrap(),
        )
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
}
#[tokio::test]
async fn desktop_permission_explicit_transport_and_replay_refusal() {
    let f = Fixture::new();
    let requests = Requests::default();
    let ui = Uuid::new_v4();
    let owner = f._guard.owner.instance_id;
    let backend = async {
        let (mut stream, _) = f.listener.accept().await.unwrap();
        let req = read(&mut stream).await;
        assert_eq!(req.command, "permissions_request");
        assert_ne!(req.session_id, Some(ui));
        assert_eq!(req.instance_id, Some(owner));
        reply(
            &mut stream,
            owner,
            json!({"microphone":"denied","systemAudio":"unknown","outcome":"completed"}),
        )
        .await;
    };
    let p = payload("request", ui);
    let (result, ()) = tokio::join!(requests.call(&f.paths, &f.runtime, &p), backend);
    assert_eq!(result.unwrap()["microphone"], "denied");
    assert!(requests
        .call(&f.paths, &f.runtime, &payload("cancel", ui))
        .await
        .is_err());
    assert!(requests
        .call(&f.paths, &f.runtime, &payload("request", ui))
        .await
        .is_err());
    assert!(std::fs::read_dir(&f.paths.data).unwrap().next().is_none());
}
#[tokio::test]
async fn desktop_permission_cancel_owns_connection_and_acks_before_success() {
    let f = Fixture::new();
    let requests = Requests::default();
    let ui = Uuid::new_v4();
    let arrived = tokio::sync::Notify::new();
    let owner = f._guard.owner.instance_id;
    assert!(requests
        .call(&f.paths, &f.runtime, &payload("cancel", ui))
        .await
        .is_err());
    let backend = async {
        let (mut original, _) = f.listener.accept().await.unwrap();
        let request = read(&mut original).await;
        arrived.notify_one();
        let (mut cancel, _) = f.listener.accept().await.unwrap();
        let message = read(&mut cancel).await;
        assert_eq!(message.command, "permissions_cancel");
        assert_eq!(message.session_id, request.session_id);
        assert!(
            original.write_all(b"x").await.is_err(),
            "original connection must close before cancel dispatch"
        );
        reply(&mut cancel, owner, json!({"outcome":"cancelled"})).await;
    };
    let cancel = async {
        arrived.notified().await;
        assert!(requests
            .call(&f.paths, &f.runtime, &payload("request", Uuid::new_v4()))
            .await
            .is_err());
        assert!(requests
            .call(&f.paths, &f.runtime, &payload("cancel", Uuid::new_v4()))
            .await
            .is_err());
        requests
            .call(&f.paths, &f.runtime, &payload("cancel", ui))
            .await
    };
    let p = payload("request", ui);
    let (original, cancel, ()) =
        tokio::join!(requests.call(&f.paths, &f.runtime, &p), cancel, backend);
    assert_eq!(original.unwrap()["outcome"], "cancelled");
    assert_eq!(cancel.unwrap()["outcome"], "cancelled");
    assert!(requests
        .call(&f.paths, &f.runtime, &payload("cancel", ui))
        .await
        .is_err());
}
#[tokio::test]
async fn desktop_permission_refuses_bad_payload_busy_and_replaced_endpoint() {
    let f = Fixture::new();
    let requests = Requests::default();
    let id = Uuid::new_v4();
    for p in [
        json!({"action":"shell","operationId":id}),
        json!({"action":"request","operationId":"bad"}),
        json!({"action":"request","operationId":id,"window":"main"}),
    ] {
        assert!(requests.call(&f.paths, &f.runtime, &p).await.is_err());
    }
    let backend = async {
        let (mut stream, _) = f.listener.accept().await.unwrap();
        read(&mut stream).await;
        std::fs::remove_file(f.runtime.path.join("service.sock")).unwrap();
        let _new = UnixListener::bind(f.runtime.path.join("service.sock")).unwrap();
        std::fs::set_permissions(
            f.runtime.path.join("service.sock"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        reply(
            &mut stream,
            f._guard.owner.instance_id,
            json!({"microphone":"authorized","systemAudio":"unknown","outcome":"completed"}),
        )
        .await;
    };
    let p = payload("request", id);
    let (result, ()) = tokio::join!(requests.call(&f.paths, &f.runtime, &p), backend);
    assert!(result.is_err());
}
#[tokio::test]
async fn desktop_permission_disconnected_backend_is_not_success() {
    let f = Fixture::new();
    let requests = Requests::default();
    let p = payload("request", Uuid::new_v4());
    let backend = async {
        let (mut stream, _) = f.listener.accept().await.unwrap();
        read(&mut stream).await;
    };
    let (result, ()) = tokio::join!(requests.call(&f.paths, &f.runtime, &p), backend);
    assert!(result.is_err());
    // Complete a bounded, private operation; there is no detached task or child.
    assert!(tokio::time::timeout(
        Duration::from_secs(1),
        requests.call(&f.paths, &f.runtime, &payload("cancel", Uuid::new_v4()))
    )
    .await
    .unwrap()
    .is_err());
}

#[tokio::test]
async fn desktop_permission_deadline_includes_stalled_compatibility_without_transport_or_writes() {
    let f = Fixture::new();
    let requests = Requests::default();
    let result = requests
        .desktop(
            &f.paths,
            &f.runtime,
            &payload("request", Uuid::new_v4()),
            std::future::pending(),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code,
        omm::core::contracts::ErrorCode::Timeout
    );
    assert!(std::fs::read_dir(&f.paths.data).unwrap().next().is_none());
    assert!(
        tokio::time::timeout(Duration::from_millis(1), f.listener.accept())
            .await
            .is_err()
    );
}

//! Actual typed Host dispatcher and Unix service transport; only platform compatibility is injected.
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
    host.permission_compatibility = Some(json!({"admissionMode":"service","writesAllowed":true}));
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

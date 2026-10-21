//! Exact compatibility→registration and recheck→transport replacement windows.
//! Every endpoint is an in-process fake under private roots; no OS permissions.
use omm::{
    application::{permissions::Requests, profile},
    core::{contracts::ErrorCode, paths::*, runtime::*},
    service::daemon::{self, Reply, Request},
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
    guard: RuntimeGuard,
    _listener: UnixListener,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("omm-gen-")
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
        let listener = Self::bind(&runtime);
        Self {
            _temp: temp,
            paths,
            runtime,
            guard,
            _listener: listener,
        }
    }
    fn bind(rt: &RuntimeLocation) -> UnixListener {
        let p = rt.path.join("service.sock");
        let l = UnixListener::bind(&p).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600)).unwrap();
        l
    }
    fn compatible(&self) -> Value {
        profile::compatible(&profile::read(&self.paths).unwrap().unwrap(), "service")
    }
    fn replace(&self) -> (Uuid, UnixListener) {
        // Controlled fake generation in this very process, preserving real owner/peer UID/PID.
        let mut owner = self.guard.owner.clone();
        owner.instance_id = Uuid::new_v4();
        write_private_replace(
            &self.runtime.path.join("owner.json"),
            &serde_json::to_vec(&owner).unwrap(),
        )
        .unwrap();
        let mut p = profile::read(&self.paths).unwrap().unwrap();
        p.generation = owner.instance_id;
        write_private_replace(
            &self.paths.state.join("desktop-owner.json"),
            &serde_json::to_vec(&p).unwrap(),
        )
        .unwrap();
        std::fs::remove_file(self.runtime.path.join("service.sock")).unwrap();
        (owner.instance_id, Self::bind(&self.runtime))
    }
}
fn payload() -> Value {
    json!({"action":"request","operationId":Uuid::new_v4()})
}
async fn observe(listener: UnixListener, owner: Uuid) -> bool {
    let Ok(Ok((mut s, _))) =
        tokio::time::timeout(Duration::from_millis(150), listener.accept()).await
    else {
        return false;
    };
    let mut b = vec![];
    s.read_to_end(&mut b).await.unwrap();
    if b.is_empty() {
        return false;
    }
    let _: Request = serde_json::from_slice(&b).unwrap();
    s.write_all(
        &serde_json::to_vec(&Reply {
            ok: true,
            data: Some(
                json!({"microphone":"denied","systemAudio":"unknown","outcome":"completed"}),
            ),
            error: None,
            service_instance: Some(owner),
        })
        .unwrap(),
    )
    .await
    .unwrap();
    s.shutdown().await.unwrap();
    true
}
#[tokio::test]
async fn permission_generation_endpoint_owner_and_profile_mismatch_before_registration_send_nothing(
) {
    for changed in ["endpoint", "owner", "profile"] {
        let f = Fixture::new();
        let checked = f.compatible();
        let requests = Requests::default();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let compatibility = async {
            match changed {
                "owner" => {
                    let mut owner = f.guard.owner.clone();
                    owner.schema_version = 2;
                    write_private_replace(
                        &f.runtime.path.join("owner.json"),
                        &serde_json::to_vec(&owner).unwrap(),
                    )
                    .unwrap();
                }
                "profile" => {
                    let mut p = profile::read(&f.paths).unwrap().unwrap();
                    p.generation = Uuid::new_v4();
                    write_private_replace(
                        &f.paths.state.join("desktop-owner.json"),
                        &serde_json::to_vec(&p).unwrap(),
                    )
                    .unwrap();
                }
                _ => {}
            }
            std::fs::remove_file(f.runtime.path.join("service.sock")).unwrap();
            tx.send(Fixture::bind(&f.runtime)).unwrap();
            checked
        };
        let p = payload();
        let (result, acted) = tokio::join!(
            requests.desktop(&f.paths, &f.runtime, &p, compatibility),
            async { observe(rx.await.unwrap(), f.guard.owner.instance_id).await }
        );
        assert!(!acted, "changed {changed} received a permission action");
        assert!(result.is_err());
    }
}
#[tokio::test]
async fn permission_generation_late_reply_and_cancel_cannot_affect_successor() {
    let f = Fixture::new();
    let requests = Requests::default();
    let old = payload();
    let checked = f.compatible();
    let backend = async {
        let (mut stream, _) = f._listener.accept().await.unwrap();
        let mut bytes = vec![];
        stream.read_to_end(&mut bytes).await.unwrap();
        let original: Request = serde_json::from_slice(&bytes).unwrap();
        let next = f.replace();
        stream.write_all(&serde_json::to_vec(&Reply { ok:true, data:Some(json!({"microphone":"denied","systemAudio":"unknown","outcome":"completed"})), error:None, service_instance:Some(f.guard.owner.instance_id) }).unwrap()).await.unwrap();
        stream.shutdown().await.unwrap();
        (next, original.session_id)
    };
    let (result, ((next, listener), original_native)) = tokio::join!(
        requests.desktop(&f.paths, &f.runtime, &old, async { checked }),
        backend
    );
    assert!(result.is_err(), "late G1 reply must not authorize G2");
    let successor = payload();
    let next_backend = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = vec![];
        stream.read_to_end(&mut bytes).await.unwrap();
        let current: Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(current.command, "permissions_request");
        assert_ne!(current.session_id, original_native);
        let late_cancel = json!({"action":"cancel","operationId":old["operationId"]});
        assert!(requests
            .desktop(&f.paths, &f.runtime, &late_cancel, std::future::pending())
            .await
            .is_err());
        stream.write_all(&serde_json::to_vec(&Reply { ok:true, data:Some(json!({"microphone":"denied","systemAudio":"unknown","outcome":"completed"})), error:None, service_instance:Some(next) }).unwrap()).await.unwrap();
        stream.shutdown().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(150), listener.accept())
                .await
                .is_err()
        );
    };
    let (result, ()) = tokio::join!(
        requests.desktop(&f.paths, &f.runtime, &successor, async { f.compatible() }),
        next_backend
    );
    assert_eq!(result.unwrap()["microphone"], "denied");
}
#[tokio::test]
async fn permission_generation_compatibility_then_replacement_never_adopts_g2() {
    let f = Fixture::new();
    let checked = f.compatible();
    let requests = Requests::default();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let compatibility = async {
        let next = f.replace();
        tx.send(next).unwrap();
        checked
    };
    let fake = async {
        let (owner, listener) = rx.await.unwrap();
        observe(listener, owner).await
    };
    let p = payload();
    let (result, acted) = tokio::join!(
        requests.desktop(&f.paths, &f.runtime, &p, compatibility),
        fake
    );
    assert!(!acted, "unchecked G2 received a permission action");
    assert!(result.is_err());
}
#[tokio::test]
async fn permission_generation_last_recheck_then_transport_replacement_sends_nothing() {
    for command in ["permissions_request", "permissions_cancel"] {
        let f = Fixture::new();
        let checked = verify_active(&f.runtime.path, &f.paths.state).unwrap();
        let (next, listener) = f.replace();
        let req = Request {
            schema_version: 1,
            command: command.into(),
            title: None,
            session_id: Some(Uuid::new_v4()),
            instance_id: Some(checked.instance_id),
            desktop_request: None,
            desktop_capability: None,
        };
        let (result, acted) = tokio::join!(
            daemon::request_message_at(&f.paths, &f.runtime, req),
            observe(listener, next)
        );
        assert!(
            !acted,
            "unchecked G2 received {command} after caller's last recheck"
        );
        assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
    }
}
#[tokio::test]
async fn permission_generation_compatibility_without_generation_cannot_borrow_current_stamp() {
    let f = Fixture::new();
    let requests = Requests::default();
    let p = payload();
    let (result, acted) = tokio::join!(
        requests.desktop(&f.paths, &f.runtime, &p, async {
            json!({"admissionMode":"service","writesAllowed":true})
        }),
        async {
            let Ok(Ok((mut s, _))) =
                tokio::time::timeout(Duration::from_millis(150), f._listener.accept()).await
            else {
                return false;
            };
            let mut b = vec![];
            s.read_to_end(&mut b).await.unwrap();
            if b.is_empty() {
                return false;
            }
            s.write_all(&serde_json::to_vec(&Reply{ok:true,data:Some(json!({"microphone":"denied","systemAudio":"unknown","outcome":"completed"})),error:None,service_instance:Some(f.guard.owner.instance_id)}).unwrap()).await.unwrap();
            s.shutdown().await.unwrap();
            true
        }
    );
    assert!(!acted);
    assert!(result.is_err());
}

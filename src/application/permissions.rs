//! One trusted desktop Host's explicit permission operation. No capture or profile writes.
use crate::{
    core::{
        contracts::*,
        paths::Paths,
        runtime::{self, RuntimeLocation},
    },
    service::daemon,
};
use serde_json::{json, Value};
use std::{collections::HashSet, os::unix::fs::MetadataExt, sync::Mutex, time::Duration};
use tokio::sync::watch;
use uuid::Uuid;
#[derive(Clone)]
struct Active {
    ui: Uuid,
    native: Uuid,
    stamp: Value,
    cancel: watch::Sender<bool>,
    done: watch::Sender<Option<bool>>,
}
#[derive(Default)]
struct State {
    active: Option<Active>,
    used: HashSet<Uuid>,
}
#[derive(Default)]
pub struct Requests {
    state: Mutex<State>,
}
fn conflict() -> OmmError {
    OmmError::new(
        ErrorCode::Conflict,
        false,
        "permission operation is stale or busy",
    )
}
fn stamp(paths: &Paths, runtime: &RuntimeLocation) -> Result<Value> {
    let owner = runtime::verify_active(&runtime.path, &paths.state)?;
    let profile = super::profile::read(paths)?.ok_or_else(conflict)?;
    if profile.generation != owner.instance_id {
        return Err(conflict());
    }
    let socket = std::fs::symlink_metadata(runtime.path.join("service.sock"))?;
    Ok(
        json!({"owner":owner,"profile":profile.profile_id,"device":socket.dev(),"inode":socket.ino()}),
    )
}
fn recheck(paths: &Paths, runtime: &RuntimeLocation, expected: &Value) -> Result<()> {
    if stamp(paths, runtime)? != *expected {
        return Err(conflict());
    }
    Ok(())
}
fn cancelled() -> Value {
    json!({"microphone":"unknown","systemAudio":"unknown","outcome":"cancelled"})
}
struct Lease<'a> {
    requests: &'a Requests,
    native: Uuid,
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let mut state = self.requests.state.lock().unwrap();
        if state
            .active
            .as_ref()
            .is_some_and(|a| a.native == self.native)
        {
            let active = state.active.take().unwrap();
            if active.done.borrow().is_none() {
                active.done.send_replace(Some(false));
            }
        }
    }
}
impl Requests {
    /// The complete trusted desktop command, including compatibility inspection.
    /// Cancel never queues a capability RPC behind the pending OS request.
    pub async fn desktop(
        &self,
        paths: &Paths,
        runtime: &RuntimeLocation,
        payload: &Value,
        compatibility: impl std::future::Future<Output = Value>,
    ) -> Result<Value> {
        tokio::time::timeout(Duration::from_secs(25), async {
            super::validation::validate_named("PermissionRequest", payload)?;
            // Capture before polling the handshake: a later stamp cannot grant a
            // different owner/endpoint the compatibility of this generation.
            let expected = if payload["action"] == "request" {
                let checked = stamp(paths, runtime)?;
                let value = compatibility.await;
                if value["admissionMode"] != "service" || value["writesAllowed"] != true {
                    return Err(OmmError::unavailable(
                        "compatible running service required for permission request",
                    ));
                }
                if value["serviceGeneration"] != checked["owner"]["instanceId"] {
                    return Err(conflict());
                }
                recheck(paths, runtime, &checked)?;
                Some(checked)
            } else {
                None
            };
            self.call(paths, runtime, payload, expected).await
        })
        .await
        .map_err(|_| {
            OmmError::new(
                ErrorCode::Timeout,
                false,
                "permission request unconfirmed; OS dialog may remain pending",
            )
        })?
    }
    async fn call(
        &self,
        paths: &Paths,
        runtime: &RuntimeLocation,
        payload: &Value,
        expected: Option<Value>,
    ) -> Result<Value> {
        super::validation::validate_named("PermissionRequest", payload)?;
        let id = super::id(payload, "operationId")?;
        if payload["action"] == "cancel" {
            let active = {
                let state = self.state.lock().unwrap();
                state
                    .active
                    .as_ref()
                    .filter(|a| a.ui == id)
                    .cloned()
                    .ok_or_else(conflict)?
            };
            recheck(paths, runtime, &active.stamp)?;
            let mut done = active.done.subscribe();
            {
                let state = self.state.lock().unwrap();
                if !state
                    .active
                    .as_ref()
                    .is_some_and(|a| a.native == active.native)
                {
                    return Err(conflict());
                }
                active.cancel.send_replace(true);
            }
            // The request owns transport teardown and the service cancellation ACK.
            // Do not claim cancellation before that finite operation has finished.
            tokio::time::timeout(Duration::from_secs(25), async {
                loop {
                    let result = *done.borrow_and_update();
                    if let Some(confirmed) = result {
                        return if confirmed {
                            Ok(())
                        } else {
                            Err(OmmError::unavailable(
                                "permission cancellation is unconfirmed",
                            ))
                        };
                    }
                    done.changed().await.map_err(|_| conflict())?;
                }
            })
            .await
            .map_err(|_| {
                OmmError::new(
                    ErrorCode::Timeout,
                    false,
                    "permission cancellation is unconfirmed",
                )
            })??;
            recheck(paths, runtime, &active.stamp)?;
            return Ok(cancelled());
        }
        if payload["action"] != "request" {
            return Err(OmmError::invalid("invalid permission action"));
        }
        let expected = expected.ok_or_else(conflict)?;
        recheck(paths, runtime, &expected)?;
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, _) = watch::channel(None);
        let active = Active {
            ui: id,
            native: Uuid::new_v4(),
            stamp: expected,
            cancel,
            done,
        };
        {
            let mut state = self.state.lock().unwrap();
            if state.active.is_some() || state.used.contains(&id) || state.used.len() >= 128 {
                return Err(conflict());
            }
            state.used.insert(id);
            state.active = Some(active.clone());
        }
        let _lease = Lease {
            requests: self,
            native: active.native,
        };
        let owner = Uuid::parse_str(
            active.stamp["owner"]["instanceId"]
                .as_str()
                .ok_or_else(conflict)?,
        )
        .map_err(|_| conflict())?;
        let message = |command: &str| daemon::Request {
            schema_version: 1,
            command: command.into(),
            title: None,
            session_id: Some(active.native),
            instance_id: Some(owner),
            desktop_request: None,
            desktop_capability: None,
        };
        let operation = async {
            recheck(paths, runtime, &active.stamp)?;
            let result = {
                // This scope owns the request socket. Cancelling closes it before
                // sending the cancel action, releasing the service's caller watch.
                let request =
                    daemon::request_message_at(paths, runtime, message("permissions_request"));
                tokio::pin!(request);
                tokio::select! { biased;
                    _=cancellation.changed()=>None,
                    result=&mut request=>Some(result),
                }
            };
            recheck(paths, runtime, &active.stamp)?;
            if result.is_none() || *cancellation.borrow() {
                daemon::request_message_at(paths, runtime, message("permissions_cancel")).await?;
                recheck(paths, runtime, &active.stamp)?;
                return Ok(cancelled());
            }
            let value = result.unwrap()?;
            let result = json!({"microphone":value["microphone"],"systemAudio":value["systemAudio"],"outcome":value["outcome"]});
            super::validation::response("permissions_request", &result)?;
            Ok(result)
        };
        let result = tokio::time::timeout(Duration::from_secs(25), operation)
            .await
            .map_err(|_| {
                OmmError::new(
                    ErrorCode::Timeout,
                    false,
                    "permission request unconfirmed; OS dialog may remain pending",
                )
            })?;
        active.done.send_replace(Some(
            result
                .as_ref()
                .is_ok_and(|value| value["outcome"] == "cancelled"),
        ));
        result
    }
}

//! Cancellation cleanup keeps associated resources alive until the owned child is reaped.
use super::contracts::{OmmError, Result};
use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex},
};
use tokio::{process::Child, sync::watch};
#[derive(Clone, Default)]
pub struct Reapers(Arc<Mutex<Vec<watch::Receiver<Option<bool>>>>>);
impl Reapers {
    /// Register a bounded adapter cleanup before its task can be cancelled.
    pub fn register_cleanup(&self) -> watch::Sender<Option<bool>> {
        let (complete, receiver) = watch::channel(None);
        self.0.lock().unwrap().push(receiver);
        complete
    }
    /// Receivers remain registered across cancellation of this wait.
    pub async fn drain(&self) -> Result<()> {
        loop {
            let receivers = self.0.lock().unwrap().clone();
            if receivers.is_empty() {
                break;
            }
            for mut receiver in receivers.iter().cloned() {
                while receiver.borrow_and_update().is_none() {
                    receiver.changed().await.map_err(|_| {
                        OmmError::unavailable(
                            "child cleanup executor stopped before exit was confirmed",
                        )
                    })?;
                }
                if *receiver.borrow() != Some(true) {
                    return Err(OmmError::unavailable(
                        "owned child exit could not be confirmed",
                    ));
                }
            }
            self.0
                .lock()
                .unwrap()
                // A cleanup task may register its child reaper while we await it.
                .retain(|receiver| !receivers.iter().any(|waited| waited.same_channel(receiver)));
        }
        Ok(())
    }
}
pub struct OwnedChild {
    child: Option<Child>,
    reapers: Reapers,
    resource: Option<Arc<dyn Send + Sync>>,
}
impl OwnedChild {
    pub fn new(child: Child, reapers: Reapers, resource: Option<Arc<dyn Send + Sync>>) -> Self {
        Self {
            child: Some(child),
            reapers,
            resource,
        }
    }
}
impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.child.as_ref().unwrap()
    }
}
impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.child.as_mut().unwrap()
    }
}
struct ResourceHold(Option<Arc<dyn Send + Sync>>);
impl Drop for ResourceHold {
    fn drop(&mut self) {
        // Executor shutdown or wait failure cannot establish child exit. Keep recovery evidence.
        std::mem::forget(self.0.take());
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if child.try_wait().is_ok_and(|exit| exit.is_some()) {
            return;
        }
        let _ = child.start_kill();
        let mut resource = ResourceHold(self.resource.take());
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let (complete, receiver) = watch::channel(None);
            self.reapers.0.lock().unwrap().push(receiver);
            runtime.spawn(async move {
                let exited = child.wait().await.is_ok();
                if exited {
                    drop(resource.0.take());
                }
                complete.send_replace(Some(exited));
            });
        }
    }
}
/// Keep the advisory audio reader lock in an owned child across a service crash.
/// The descriptor carries no credentials and remains locked until the child exits.
pub fn inherit_audio_reader(command: &mut tokio::process::Command, file: &std::fs::File) {
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    unsafe {
        command.pre_exec(move || {
            if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

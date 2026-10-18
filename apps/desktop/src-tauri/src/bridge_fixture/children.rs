//! Fixture-only child retention. No signal, blocking Drop, or detached cleanup task.
use std::{
    ops::{Deref, DerefMut},
    process::{Child, ExitStatus},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
        LazyLock, Mutex,
    },
};
static LIVE: AtomicUsize = AtomicUsize::new(0);
static CORE_PENDING: AtomicBool = AtomicBool::new(false);
static RETAINED: Mutex<Vec<Child>> = Mutex::new(Vec::new());
static HANDOFF: LazyLock<(Sender<Child>, Mutex<Receiver<Child>>)> = LazyLock::new(|| {
    let (send, receive) = mpsc::channel();
    (send, Mutex::new(receive))
});
pub struct ManagedChild {
    child: Option<Child>,
    live: bool,
}
impl ManagedChild {
    pub fn new(child: Child) -> Self {
        LazyLock::force(&HANDOFF);
        LIVE.fetch_add(1, Ordering::SeqCst);
        Self {
            child: Some(child),
            live: true,
        }
    }
    fn completed(&mut self) {
        if self.live {
            self.live = false;
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let result = self.child.as_mut().unwrap().try_wait()?;
        if result.is_some() {
            self.completed();
        }
        Ok(result)
    }
    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let result = self.child.as_mut().unwrap().wait()?;
        self.completed();
        Ok(result)
    }
}
impl Deref for ManagedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.child.as_ref().unwrap()
    }
}
impl DerefMut for ManagedChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.child.as_mut().unwrap()
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        if self.try_wait().ok().flatten().is_none() {
            // Unbounded send is nonblocking; the receiver remains owned by the host.
            HANDOFF
                .0
                .send(self.child.take().unwrap())
                .expect("host retention receiver is permanent");
        }
    }
}
pub fn core_pending(value: bool) {
    CORE_PENDING.store(value, Ordering::SeqCst);
}
pub fn exit_blocked() -> bool {
    let Ok(mut children) = RETAINED.try_lock() else {
        return true;
    };
    let Ok(receive) = HANDOFF.1.try_lock() else {
        return true;
    };
    children.extend(receive.try_iter());
    children.retain_mut(|child| {
        if child.try_wait().ok().flatten().is_some() {
            LIVE.fetch_sub(1, Ordering::SeqCst);
            false
        } else {
            true
        }
    });
    CORE_PENDING.load(Ordering::SeqCst) || LIVE.load(Ordering::SeqCst) != 0
}
pub fn retained_ids() -> Vec<u32> {
    let _ = exit_blocked();
    RETAINED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(Child::id)
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    #[test]
    fn dropped_live_child_is_retained_without_signal_and_can_cooperate() {
        assert!(!exit_blocked());
        core_pending(true);
        assert!(exit_blocked());
        core_pending(false);
        let child = Command::new("/bin/sh")
            .args(["-c", "read answer; test \"$answer\" = quit"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let guard = ManagedChild::new(child);
        let pid = guard.id();
        assert!(exit_blocked());
        drop(guard);
        assert!(exit_blocked());
        assert!(retained_ids().contains(&pid));
        {
            let mut all = RETAINED.lock().unwrap();
            let child = all.iter_mut().find(|c| c.id() == pid).unwrap();
            assert!(child.try_wait().unwrap().is_none());
            child.stdin.as_mut().unwrap().write_all(b"quit\n").unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while exit_blocked() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(!retained_ids().contains(&pid));
        let mut completed = ManagedChild::new(Command::new("/usr/bin/true").spawn().unwrap());
        assert!(completed.wait().unwrap().success());
        drop(completed);
        assert!(!exit_blocked());
    }
}

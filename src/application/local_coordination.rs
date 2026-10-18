//! Native callers serialize only local profile control/probes; OS admission locks stay nonblocking.
use crate::core::{contracts::*, paths::Paths};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Key by the stable canonical state path, not its inode: initialization/replacement must
/// not create a second gate. Valid profile roots already reject aliases and traversal.
/// No directories, persistent state or OS locks are created by this coordinator.
pub async fn enter(paths: &Paths, budget: Duration) -> Result<OwnedMutexGuard<()>> {
    super::profile::profile_id(paths)?;
    static GATES: OnceLock<Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let gate = {
        let mut gates = GATES
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| OmmError::unavailable("local profile coordinator unavailable"))?;
        gates.retain(|_, gate| gate.strong_count() != 0);
        match gates.get(&paths.state).and_then(Weak::upgrade) {
            Some(gate) => gate,
            None => {
                let gate = Arc::new(AsyncMutex::new(()));
                gates.insert(paths.state.clone(), Arc::downgrade(&gate));
                gate
            }
        }
    };
    tokio::time::timeout(budget, gate.lock_owned())
        .await
        .map_err(|_| {
            OmmError::new(
                ErrorCode::Conflict,
                true,
                "local profile check or control is busy",
            )
        })
}

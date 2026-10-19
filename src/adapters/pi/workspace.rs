use crate::core::{contracts::*, paths::*, store::Store};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};
use uuid::Uuid;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    schema_version: u32,
    workspace_id: Uuid,
    instance_id: Uuid,
    task_id: Uuid,
    session_id: Uuid,
    attempt: u32,
    state: PathBuf,
    uid: u32,
    owner_pid: u32,
    child_pid: Option<u32>,
    #[serde(default = "legacy_background_unverified")]
    background_unverified: bool,
    device: u64,
    inode: u64,
}
fn legacy_background_unverified() -> bool {
    true
}
pub(super) struct Workspace {
    pub path: PathBuf,
    lock: File,
    owner: Mutex<Owner>,
}
fn private_canonical(path: &Path) -> Result<()> {
    validate_private_dir(path)?;
    if path.canonicalize()? != path {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "Pi workspace path is not canonical",
        ));
    }
    Ok(())
}
fn read_owner(path: &Path) -> Result<Owner> {
    let file = path.join("owner.json");
    validate_private_file(&file)?;
    if fs::symlink_metadata(&file)?.nlink() != 1 {
        return Err(OmmError::invalid("Pi owner file is not private"));
    }
    let mut bytes = vec![];
    File::open(file)?.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(OmmError::invalid("Pi owner record exceeds limit"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn verify_owner(path: &Path, owner: &Owner, state: &Path) -> Result<()> {
    private_canonical(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if owner.schema_version != 1
        || owner.uid != unsafe { libc::geteuid() }
        || owner.state != state
        || owner.attempt == 0
        || owner.workspace_id.to_string() != path.file_name().unwrap_or_default().to_string_lossy()
        || owner.device != metadata.dev()
        || owner.inode != metadata.ino()
    {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "Pi workspace ownership does not match",
        ));
    }
    Ok(())
}
fn open_lock(path: &Path) -> Result<File> {
    let lock = path.join("workspace.lock");
    validate_private_file(&lock)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(lock)?;
    if file.metadata()?.nlink() != 1 {
        return Err(OmmError::invalid("Pi workspace lock is not private"));
    }
    Ok(file)
}
impl Workspace {
    pub fn create(paths: &Paths, context: &TaskContext, instance_id: Uuid) -> Result<Self> {
        private_canonical(&paths.cache)?;
        let root = paths.cache.join("pi");
        private_dir(&root)?;
        private_canonical(&root)?;
        let id = Uuid::new_v4();
        let path = root.join(id.to_string());
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path.join("workspace.lock"))?;
        lock.try_lock_exclusive()?;
        let metadata = fs::symlink_metadata(&path)?;
        let owner = Owner {
            schema_version: 1,
            workspace_id: id,
            instance_id,
            task_id: context.task_id,
            session_id: context.session_id,
            attempt: context.attempt,
            state: paths.state.canonicalize()?,
            uid: unsafe { libc::geteuid() },
            owner_pid: std::process::id(),
            child_pid: None,
            background_unverified: false,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        write_private_new(&path.join("owner.json"), &serde_json::to_vec(&owner)?)?;
        let workspace = Self {
            path,
            lock,
            owner: Mutex::new(owner),
        };
        private_dir(&workspace.path.join("home"))?;
        Ok(workspace)
    }
    pub fn prepare_child(&self) -> Result<()> {
        // Persist uncertainty before every spawn, including subsequent summary chunks.
        self.update_child(None)
    }
    pub fn record_child(&self, child_pid: u32) -> Result<()> {
        self.update_child(Some(child_pid))
    }
    fn update_child(&self, child_pid: Option<u32>) -> Result<()> {
        let mut owner = self.owner.lock().unwrap();
        if read_owner(&self.path)? != *owner {
            return Err(OmmError::invalid("Pi workspace owner changed"));
        }
        let mut next = owner.clone();
        next.child_pid = child_pid;
        next.background_unverified = true;
        write_private_replace(&self.path.join("owner.json"), &serde_json::to_vec(&next)?)?;
        *owner = next;
        Ok(())
    }
    pub fn cleanup(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let owner = self.owner.lock().unwrap();
        verify_owner(&self.path, &owner, &owner.state)?;
        if read_owner(&self.path)? != *owner {
            return Err(OmmError::invalid("Pi workspace owner changed"));
        }
        let actual = open_lock(&self.path)?.metadata()?;
        let held = self.lock.metadata()?;
        if actual.dev() != held.dev() || actual.ino() != held.ino() {
            return Err(OmmError::invalid("Pi workspace lock changed"));
        }
        if owner.background_unverified {
            tracing::warn!(task_id = %owner.task_id, workspace_id = %owner.workspace_id, "Pi owned processes settled; background activity unverified; private workspace retained");
            return Ok(());
        }
        fs::remove_dir_all(&self.path)?;
        Ok(())
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recovery {
    pub removed: usize,
    pub retained_live: usize,
    pub retained_unknown: usize,
}
/// Spawned or legacy Pi workspaces never establish complete background coverage; no PID is signalled.
pub fn recover(paths: &Paths, store: &Store) -> Result<Recovery> {
    let root = paths.cache.join("pi");
    let mut report = Recovery::default();
    if fs::symlink_metadata(&root).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return Ok(report);
    }
    private_canonical(&paths.cache)?;
    private_canonical(&root)?;
    let state = paths.state.canonicalize()?;
    for entry in fs::read_dir(&root)?.take(4096) {
        let path = entry?.path();
        let candidate = (|| -> Result<(Owner, File)> {
            let owner = read_owner(&path)?;
            verify_owner(&path, &owner, &state)?;
            let task = store.task(owner.task_id)?;
            if task.session_id != owner.session_id || task.attempt != owner.attempt {
                return Err(OmmError::invalid(
                    "Pi workspace task ownership does not match",
                ));
            }
            Ok((owner, open_lock(&path)?))
        })();
        let Ok((owner, lock)) = candidate else {
            report.retained_unknown += 1;
            continue;
        };
        match lock.try_lock_exclusive() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                report.retained_live += 1;
                continue;
            }
            Err(_) => {
                report.retained_unknown += 1;
                continue;
            }
            Ok(()) => {}
        }
        // A free lock or a dead recorded PID cannot cover launcher-created background work.
        // Missing legacy flags are deliberately conservative too.
        report.retained_unknown += 1;
        tracing::warn!(task_id = %owner.task_id, workspace_id = %owner.workspace_id, "Pi background activity unverified; recovery retained private workspace");
    }
    Ok(report)
}

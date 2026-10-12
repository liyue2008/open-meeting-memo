use super::{
    contracts::*,
    paths::{validate_private_dir, validate_private_file, write_private_replace},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

const REPLACEMENT_WARNING: &str = "using macOS user-temporary OMM replacement (not XDG_RUNTIME_DIR): best-effort cleanup; no immediate cleanup guarantee after SIGKILL or multiple logins";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLocation {
    pub path: PathBuf,
    pub source: String,
    pub lifecycle: String,
    pub warnings: Vec<String>,
}
impl RuntimeLocation {
    /// The supplied root is an injected OS provider, not an environment locator.
    pub fn resolve(
        env: &BTreeMap<String, String>,
        system_temp: impl FnOnce() -> Result<PathBuf>,
    ) -> Result<Self> {
        let reason = match env.get("XDG_RUNTIME_DIR").filter(|s| !s.is_empty()) {
            Some(value) => {
                let root = Path::new(value);
                reject_noncanonical_components(root)?;
                match fs::symlink_metadata(root) {
                    Ok(_) => {
                        validate_root(root)?;
                        return Self::location(root, "explicit_xdg", "external_provider", vec![]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        "explicit XDG_RUNTIME_DIR does not exist; replacement selected"
                    }
                    Err(_) => {
                        return Err(OmmError::new(
                            ErrorCode::Permission,
                            false,
                            "explicit XDG_RUNTIME_DIR cannot be safely inspected; fallback refused",
                        ))
                    }
                }
            }
            None => "XDG_RUNTIME_DIR is unset or empty; replacement selected",
        };
        let root = system_temp()?;
        reject_noncanonical_components(&root)?;
        validate_root(&root)?;
        Self::location(
            &root,
            "macos_replacement",
            "best_effort",
            vec![reason.into(), REPLACEMENT_WARNING.into()],
        )
    }
    fn location(root: &Path, source: &str, lifecycle: &str, warnings: Vec<String>) -> Result<Self> {
        let path = root.join("omm");
        if ["service.sock", "capture/audio_daemon.sock"]
            .iter()
            .any(|s| path.join(s).as_os_str().as_encoded_bytes().len() >= 104)
        {
            return Err(OmmError::invalid(
                "runtime socket path is too long; choose a shorter valid explicit XDG_RUNTIME_DIR",
            ));
        }
        if entry_exists(&path)? {
            validate_private_dir(&path)?;
            reject_noncanonical_components(&path)?;
        }
        Ok(Self {
            path,
            source: source.into(),
            lifecycle: lifecycle.into(),
            warnings,
        })
    }
}
fn entry_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "runtime entry cannot be safely inspected; cleanup refused",
        )),
    }
}
fn reject_noncanonical_components(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "runtime must be absolute without traversal; fallback refused",
        ));
    }
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(OmmError::new(
                    ErrorCode::Permission,
                    false,
                    "runtime symlink refused; fallback refused",
                ))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(OmmError::new(
                    ErrorCode::Permission,
                    false,
                    "runtime ancestor cannot be safely inspected",
                ))
            }
        }
    }
    Ok(())
}
fn validate_root(root: &Path) -> Result<()> {
    validate_private_dir(root)?;
    if root.canonicalize()? != root {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "runtime root must be canonical",
        ));
    }
    #[cfg(target_os = "macos")]
    {
        let path = std::ffi::CString::new(root.as_os_str().as_encoded_bytes())
            .map_err(|_| OmmError::invalid("runtime path contains NUL"))?;
        let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } != 0 {
            return Err(OmmError::unavailable("cannot inspect runtime filesystem"));
        }
        if unsafe { info.assume_init() }.f_flags & libc::MNT_LOCAL as u32 == 0 {
            return Err(OmmError::unavailable(
                "runtime must use a local filesystem; fallback refused",
            ));
        }
    }
    Ok(())
}
pub fn system_user_temp() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let size =
            unsafe { libc::confstr(libc::_CS_DARWIN_USER_TEMP_DIR, std::ptr::null_mut(), 0) };
        if !(2..=4096).contains(&size) {
            return Err(OmmError::unavailable(
                "macOS user temporary region unavailable",
            ));
        }
        let mut buf = vec![0u8; size];
        if unsafe {
            libc::confstr(
                libc::_CS_DARWIN_USER_TEMP_DIR,
                buf.as_mut_ptr().cast(),
                size,
            )
        } != size
            || buf.last() != Some(&0)
        {
            return Err(OmmError::unavailable(
                "macOS user temporary region changed during lookup",
            ));
        }
        buf.pop();
        let original = PathBuf::from(std::ffi::OsString::from_vec(buf));
        validate_private_dir(&original)?; // Reject a replaced leaf; only OS-owned ancestor aliases are canonicalized.
        Ok(original.canonicalize()?)
    }
    #[cfg(not(target_os = "macos"))]
    Err(OmmError::unavailable(
        "automatic runtime replacement is only validated on macOS",
    ))
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeOwner {
    pub schema_version: u32,
    pub uid: u32,
    pub pid: i32,
    pub instance_id: Uuid,
    pub runtime: PathBuf,
    pub state: PathBuf,
}
pub struct RuntimeGuard {
    pub owner: RuntimeOwner,
    _runtime_lock: fs::File,
    _state_lock: fs::File,
}
fn locked_file(path: &Path) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    validate_lock(path, &file)?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
        OmmError::new(
            ErrorCode::Conflict,
            false,
            "service runtime or persistent STATE already has an active instance",
        )
    })?;
    Ok(file)
}
fn validate_lock(path: &Path, file: &fs::File) -> Result<()> {
    validate_private_file(path)?;
    let m = file.metadata()?;
    let p = fs::symlink_metadata(path)?;
    if m.ino() != p.ino() || m.dev() != p.dev() || m.nlink() != 1 || m.mode() & 0o777 != 0o600 {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "service lock identity or permissions unsafe",
        ));
    }
    Ok(())
}
pub fn read_owner(runtime: &Path, state: &Path) -> Result<RuntimeOwner> {
    validate_private_dir(runtime)?;
    let path = runtime.join("owner.json");
    validate_private_file(&path)?;
    if fs::metadata(&path)?.len() > 4096 || fs::metadata(&path)?.nlink() != 1 {
        return Err(OmmError::new(
            ErrorCode::Corrupt,
            false,
            "runtime owner record invalid",
        ));
    }
    let owner: RuntimeOwner = serde_json::from_slice(&fs::read(path)?)
        .map_err(|_| OmmError::new(ErrorCode::Corrupt, false, "runtime owner record invalid"))?;
    if owner.schema_version != 1
        || owner.uid != unsafe { libc::geteuid() }
        || owner.pid <= 0
        || owner.runtime != runtime
        || owner.state != state
    {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "runtime owner does not match selected user/runtime/STATE",
        ));
    }
    Ok(owner)
}
pub fn owner_is_live(owner: &RuntimeOwner) -> bool {
    unsafe {
        libc::kill(owner.pid, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
}
impl RuntimeGuard {
    pub fn acquire(location: &RuntimeLocation, state: &Path) -> Result<Self> {
        super::paths::private_dir(&location.path)?;
        super::paths::private_dir(state)?;
        let state_lock = locked_file(&state.join("service.lock"))?;
        let runtime_lock = locked_file(&location.path.join("service.lock"))?;
        let marker = location.path.join("owner.json");
        let socket = location.path.join("service.sock");
        if entry_exists(&marker)? {
            let old = read_owner(&location.path, state)?;
            if owner_is_live(&old) {
                return Err(OmmError::new(
                    ErrorCode::Conflict,
                    false,
                    "previous runtime owner is still alive; cleanup refused",
                ));
            }
            if entry_exists(&socket)? {
                validate_socket(&socket)?;
                fs::remove_file(&socket)?;
            }
            fs::remove_file(&marker)?;
        } else if entry_exists(&socket)? {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "unidentified runtime socket; automatic deletion refused",
            ));
        }
        let owner = RuntimeOwner {
            schema_version: 1,
            uid: unsafe { libc::geteuid() },
            pid: std::process::id() as i32,
            instance_id: Uuid::new_v4(),
            runtime: location.path.clone(),
            state: state.to_owned(),
        };
        write_private_replace(&marker, &serde_json::to_vec(&owner)?)?;
        Ok(Self {
            owner,
            _runtime_lock: runtime_lock,
            _state_lock: state_lock,
        })
    }
}
impl Drop for RuntimeGuard {
    fn drop(&mut self) {
        if read_owner(&self.owner.runtime, &self.owner.state).is_ok_and(|v| v == self.owner) {
            let socket = self.owner.runtime.join("service.sock");
            if validate_socket(&socket).is_ok() {
                let _ = fs::remove_file(socket);
            }
            let _ = fs::remove_file(self.owner.runtime.join("owner.json"));
        }
        // Stable lock inodes remain: unlinking them would split waiting/new instances.
    }
}
pub fn validate_socket(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.file_type().is_socket()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o777 != 0o600
        || m.nlink() != 1
    {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "service socket identity or permissions unsafe",
        ));
    }
    Ok(())
}
pub fn verify_active(runtime: &Path, state: &Path) -> Result<RuntimeOwner> {
    let owner = read_owner(runtime, state)?;
    if !owner_is_live(&owner) {
        return Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            "service runtime owner is no longer alive",
        ));
    }
    let path = runtime.join("service.lock");
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    validate_lock(&path, &file)?;
    match fs2::FileExt::try_lock_shared(&file) {
        Ok(()) => {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "service runtime process lock is not held",
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(_) => {
            return Err(OmmError::unavailable(
                "service process lock cannot be verified",
            ))
        }
    }
    validate_socket(&runtime.join("service.sock"))?;
    Ok(owner)
}

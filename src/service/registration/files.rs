//! Registration-local file anchors; the stable lock is never unlinked or replaced.
use crate::core::contracts::*;
use std::{
    ffi::CString,
    fs::{File, Metadata, OpenOptions},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
const LOCK: &str = ".org.openmeetingmemo.service.registration.lock";
fn changed() -> OmmError {
    OmmError::new(
        ErrorCode::Conflict,
        false,
        "registration identity changed; preserve files and review before retry",
    )
}
#[derive(Clone, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
    uid: u32,
    mode: u32,
}
impl Identity {
    fn of(m: &Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            uid: m.uid(),
            mode: m.mode(),
        }
    }
}
#[derive(Clone)]
pub(super) struct Snapshot {
    identity: Identity,
    pub bytes: Vec<u8>,
}
pub(super) struct Directory {
    anchors: Vec<(PathBuf, File)>,
    lock: File,
}
impl Directory {
    pub fn open(home: &Path) -> Result<Self> {
        let mut anchors = Vec::new();
        for path in [
            home.to_path_buf(),
            home.join("Library"),
            home.join("Library/LaunchAgents"),
        ] {
            let f = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_DIRECTORY)
                .open(&path)?;
            let m = f.metadata()?;
            if !m.is_dir()
                || m.uid() != unsafe { libc::geteuid() }
                || m.mode() & 0o022 != 0
                || path.canonicalize()? != path
            {
                return Err(changed());
            }
            anchors.push((path, f));
        }
        let fd = anchors.last().unwrap().1.as_raw_fd();
        let name = CString::new(LOCK).unwrap();
        let raw = unsafe {
            libc::openat(
                fd,
                name.as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
                0o600,
            )
        };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let lock = unsafe { File::from_raw_fd(raw) };
        validate_file(&lock.metadata()?)?;
        if lock.metadata()?.nlink() != 1 {
            return Err(changed());
        }
        // Nonblocking conflict keeps Tokio threads responsive; callers explicitly retry after review.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "another service registration operation is active; retry after it completes",
            ));
        }
        let this = Self { anchors, lock };
        this.check()?;
        Ok(this)
    }
    fn fd(&self) -> i32 {
        self.anchors.last().unwrap().1.as_raw_fd()
    }
    fn name(name: &str) -> Result<CString> {
        CString::new(name).map_err(|_| OmmError::invalid("invalid registration filename"))
    }
    fn open_file(&self, name: &str) -> Result<Option<File>> {
        let name = Self::name(name)?;
        let raw = unsafe {
            libc::openat(
                self.fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        if raw < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(e.into());
        }
        let f = unsafe { File::from_raw_fd(raw) };
        validate_file(&f.metadata()?)?;
        Ok(Some(f))
    }
    pub fn check(&self) -> Result<()> {
        for (path, f) in &self.anchors {
            let m = std::fs::symlink_metadata(path)?;
            if Identity::of(&m) != Identity::of(&f.metadata()?)
                || !m.is_dir()
                || m.mode() & 0o022 != 0
                || path.canonicalize()? != *path
            {
                return Err(changed());
            }
        }
        let named = self.open_file(LOCK)?.ok_or_else(changed)?;
        if Identity::of(&named.metadata()?) != Identity::of(&self.lock.metadata()?)
            || named.metadata()?.nlink() != 1
        {
            return Err(changed());
        }
        Ok(())
    }
    pub fn snapshot(&self, name: &str) -> Result<Option<Snapshot>> {
        self.check()?;
        let Some(f) = self.open_file(name)? else {
            return Ok(None);
        };
        let before = Identity::of(&f.metadata()?);
        if f.metadata()?.len() > 64 * 1024 {
            return Err(OmmError::invalid("service profile exceeds limit"));
        }
        let mut bytes = Vec::new();
        f.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 64 * 1024 {
            return Err(OmmError::invalid("service profile exceeds limit"));
        }
        let named = self.open_file(name)?.ok_or_else(changed)?;
        if Identity::of(&named.metadata()?) != before {
            return Err(changed());
        }
        Ok(Some(Snapshot {
            identity: before,
            bytes,
        }))
    }
    pub fn verify(&self, name: &str, expected: Option<&Snapshot>) -> Result<()> {
        match (self.snapshot(name)?, expected) {
            (None, None) => Ok(()),
            (Some(got), Some(old)) if got.identity == old.identity && got.bytes == old.bytes => {
                Ok(())
            }
            _ => Err(changed()),
        }
    }
    pub fn publish(
        &self,
        name: &str,
        expected: Option<&Snapshot>,
        bytes: &[u8],
    ) -> Result<Snapshot> {
        self.verify(name, expected)?;
        let temp = Self::name(&format!(".omm-registration-{}.tmp", uuid::Uuid::new_v4()))?;
        let raw = unsafe {
            libc::openat(
                self.fd(),
                temp.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut f = unsafe { File::from_raw_fd(raw) };
        let result = (|| {
            f.write_all(bytes)?;
            f.sync_all()?;
            let identity = Identity::of(&f.metadata()?);
            validate_file(&f.metadata()?)?;
            self.verify(name, expected)?;
            let tmp_named = self
                .open_file(temp.to_str().unwrap())?
                .ok_or_else(changed)?;
            if Identity::of(&tmp_named.metadata()?) != identity {
                return Err(changed());
            }
            let dest = Self::name(name)?;
            // Checks narrow same-user races, not a privilege boundary or atomic compare-and-swap.
            let rc = unsafe {
                if expected.is_some() {
                    libc::renameat(self.fd(), temp.as_ptr(), self.fd(), dest.as_ptr())
                } else {
                    libc::linkat(self.fd(), temp.as_ptr(), self.fd(), dest.as_ptr(), 0)
                }
            };
            if rc != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            self.anchors.last().unwrap().1.sync_all()?;
            Ok(Snapshot {
                identity,
                bytes: bytes.to_vec(),
            })
        })();
        // Remove only our temporary inode, never a replacement or the stable operation lock.
        if let Ok(Some(named)) = self.open_file(temp.to_str().unwrap()) {
            if Identity::of(&named.metadata()?) == Identity::of(&f.metadata()?)
                && unsafe { libc::unlinkat(self.fd(), temp.as_ptr(), 0) } == 0
            {
                self.anchors.last().unwrap().1.sync_all()?;
            }
        }
        result
    }
    pub fn remove(&self, name: &str, expected: &Snapshot) -> Result<()> {
        self.verify(name, Some(expected))?;
        let name = Self::name(name)?;
        if unsafe { libc::unlinkat(self.fd(), name.as_ptr(), 0) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        self.anchors.last().unwrap().1.sync_all()?;
        Ok(())
    }
}
fn validate_file(m: &Metadata) -> Result<()> {
    if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o777 != 0o600 {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "registration file ownership, type or permissions unsafe",
        ));
    }
    Ok(())
}

//! All configuration writers serialize on one durable inode, then compare the raw-file hash.
use super::{contracts::*, paths::*, store::hash_bytes};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
};
pub struct ConfigTransaction {
    _lock: File,
    pub bytes: Option<Vec<u8>>,
    pub revision: Option<String>,
}
impl ConfigTransaction {
    pub fn acquire(paths: &Paths) -> Result<Self> {
        private_dir(&paths.config)?;
        let path = paths.config.join("config-writers.lock");
        let lock = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        validate_private_file(&path)?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| {
            OmmError::new(ErrorCode::Conflict, true, "configuration writer is busy")
        })?;
        let path = paths.config.join("config.json");
        let bytes = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
        {
            Ok(mut file) => {
                validate_private_file(&path)?;
                if file.metadata()?.len() > 128 * 1024 {
                    return Err(OmmError::invalid("configuration exceeds budget"));
                }
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Some(bytes)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let revision = bytes.as_deref().map(hash_bytes);
        Ok(Self {
            _lock: lock,
            bytes,
            revision,
        })
    }
    pub fn compare(&self, expected: &Option<String>) -> Result<()> {
        if &self.revision != expected {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "configuration changed; reload before saving",
            ));
        }
        Ok(())
    }
    pub fn publish(&self, paths: &Paths, bytes: &[u8]) -> Result<()> {
        let path = paths.config.join("config.json");
        let current = if path.try_exists()? {
            validate_private_file(&path)?;
            Some(hash_bytes(&std::fs::read(&path)?))
        } else {
            None
        };
        if current != self.revision {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "configuration changed outside writer lock",
            ));
        }
        write_private_replace(&path, bytes)
    }
}

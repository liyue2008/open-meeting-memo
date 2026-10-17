//! Cleanup ownership is independent of a selection's generation and commit deadline.
use super::*;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    version: u32,
    handle: String,
    profile_id: String,
    expires_at_ms: i64,
}
fn canonical_dir(path: &Path) -> Result<()> {
    validate_private_dir(path)?;
    if path.canonicalize()? != path {
        return Err(OmmError::invalid("import directory is not canonical"));
    }
    Ok(())
}
fn file(path: &Path) -> Result<()> {
    validate_private_file(path)?;
    let metadata = std::fs::symlink_metadata(path)?;
    if path.canonicalize()? != path || metadata.nlink() != 1 || metadata.mode() & 0o777 != 0o600 {
        return Err(OmmError::invalid(
            "import file identity or permissions unsafe",
        ));
    }
    Ok(())
}
pub(super) fn lock(paths: &Paths) -> Result<Option<File>> {
    canonical_dir(&paths.cache)?;
    let path = paths.cache.join("desktop-imports.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    self::file(&path)?;
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub(super) fn required_lock(paths: &Paths) -> Result<File> {
    lock(paths)?.ok_or_else(|| OmmError::new(ErrorCode::Conflict, true, "import staging is in use"))
}
pub(super) fn create(paths: &Paths, owner: &ProfileOwner, handle: &str) -> Result<()> {
    if owner.profile_id != super::super::profile::profile_id(paths)? {
        return Err(OmmError::invalid("import owner belongs to another profile"));
    }
    private_dir(&root(paths))?;
    canonical_dir(&root(paths))?;
    let dir = directory(paths, handle)?;
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    let record = Ownership {
        version: 1,
        handle: handle.into(),
        profile_id: owner.profile_id.clone(),
        expires_at_ms: chrono::Utc::now().timestamp_millis() + 900_000,
    };
    write_private_new(&dir.join("ownership.json"), &serde_json::to_vec(&record)?)
}
fn record(paths: &Paths, owner: &ProfileOwner, handle: &str) -> Result<i64> {
    canonical_dir(&paths.cache)?;
    canonical_dir(&root(paths))?;
    let dir = directory(paths, handle)?;
    canonical_dir(&dir)?;
    let ownership = dir.join("ownership.json");
    if ownership.try_exists()? {
        file(&ownership)?;
        if std::fs::metadata(&ownership)?.len() > 8192 {
            return Err(OmmError::invalid("import ownership exceeds budget"));
        }
        let record: Ownership = serde_json::from_slice(&std::fs::read(ownership)?)?;
        if record.version != 1 || record.handle != handle || record.profile_id != owner.profile_id {
            return Err(OmmError::invalid("import ownership mismatch"));
        }
        return Ok(record.expires_at_ms);
    }
    // Older, already-published selections prove their profile ownership too.
    let selection = selection(paths, owner, handle)?;
    Ok(selection.expires_at_ms)
}
pub(super) fn selection(paths: &Paths, owner: &ProfileOwner, handle: &str) -> Result<Selection> {
    canonical_dir(&paths.cache)?;
    canonical_dir(&root(paths))?;
    let dir = directory(paths, handle)?;
    canonical_dir(&dir)?;
    let path = dir.join("selection.json");
    file(&path)?;
    if std::fs::metadata(&path)?.len() > 8192 {
        return Err(OmmError::invalid("import selection exceeds budget"));
    }
    let selection: Selection = serde_json::from_slice(&std::fs::read(path)?)?;
    if selection.handle != handle || selection.profile_id != owner.profile_id {
        return Err(OmmError::invalid("import selection ownership mismatch"));
    }
    Ok(selection)
}
pub(super) fn remove(paths: &Paths, owner: &ProfileOwner, handle: &str) -> Result<()> {
    let dir = directory(paths, handle)?;
    // An absent selection is already released; never infer anything about final audio.
    if !std::fs::symlink_metadata(&dir).map(|_| true).or_else(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(e)
        }
    })? {
        return Ok(());
    }
    record(paths, owner, handle)?;
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        if ![
            "source",
            "imported-mix.wav",
            "selection.json",
            "ownership.json",
        ]
        .iter()
        .any(|name| entry.file_name() == *name)
        {
            return Err(OmmError::invalid("unknown import staging content retained"));
        }
        file(&entry.path())?;
        files.push(entry.path());
    }
    if dir.join("selection.json").try_exists()? {
        promotion::abandon(paths, owner, &selection(paths, owner, handle)?)?;
    }
    // Validate the complete set before deleting any bytes, and remove ownership last.
    files.sort_by_key(|p| {
        p.file_name()
            .is_some_and(|n| n == "ownership.json" || n == "selection.json")
    });
    for path in files {
        std::fs::remove_file(path)?;
    }
    std::fs::remove_dir(&dir)?;
    sync_dir(&root(paths))
}
pub(super) fn reap_locked(paths: &Paths, owner: &ProfileOwner, now: i64) -> Result<Value> {
    if !root(paths).try_exists()? {
        return Ok(json!({"removed":0,"retained":0}));
    }
    canonical_dir(&root(paths))?;
    let (mut removed, mut retained) = (0, 0);
    for entry in std::fs::read_dir(root(paths))? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(handle) = name.to_str() else {
            retained += 1;
            continue;
        };
        let expiry = record(paths, owner, handle).and_then(|expiry| {
            if entry.path().join("selection.json").try_exists()? {
                Ok(selection(paths, owner, handle)?.expires_at_ms.min(expiry))
            } else {
                Ok(expiry)
            }
        });
        match expiry {
            Ok(expires) if expires < now => {
                if remove(paths, owner, handle).is_ok() {
                    removed += 1
                } else {
                    retained += 1
                }
            }
            Ok(_) => {}
            Err(_) => retained += 1,
        }
    }
    Ok(json!({"removed":removed,"retained":retained}))
}

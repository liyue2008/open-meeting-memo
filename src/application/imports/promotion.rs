//! A selection-bound promotion record permits resuming only its own interrupted copy.
use super::*;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
pub(super) fn promote(
    paths: &Paths,
    owner: &ProfileOwner,
    selection: &Selection,
    source: &Path,
    dir: &Path,
) -> Result<PathBuf> {
    let parent = dir.parent().unwrap();
    private_dir(parent)?;
    if parent.canonicalize()? != parent {
        return Err(OmmError::invalid(
            "import promotion parent is not canonical",
        ));
    }
    let record = json!({"version":1,"profileId":owner.profile_id,"handle":selection.handle,"meetingId":selection.meeting_id,"mixHash":selection.mix_hash});
    if !dir.try_exists()? {
        std::fs::DirBuilder::new().mode(0o700).create(dir)?;
        write_private_new(
            &dir.join("import-promotion.json"),
            &serde_json::to_vec(&record)?,
        )?;
        sync_dir(dir)?;
    }
    validate_private_dir(dir)?;
    if dir.canonicalize()? != dir || !dir.starts_with(paths.data.join("meetings/imports")) {
        return Err(OmmError::invalid("unsafe import promotion directory"));
    }
    let marker = dir.join("import-promotion.json");
    validate_file(&marker)?;
    if std::fs::metadata(&marker)?.len() > 8192
        || serde_json::from_slice::<Value>(&std::fs::read(&marker)?)? != record
    {
        return Err(OmmError::invalid("import promotion ownership mismatch"));
    }
    let target = dir.join("imported-mix.wav");
    let partial = dir.join(".import-copy");
    if target.try_exists()? && std::fs::symlink_metadata(&partial).is_ok() {
        let a = std::fs::symlink_metadata(&target)?;
        let b = std::fs::symlink_metadata(&partial)?;
        if a.is_file()
            && b.is_file()
            && a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.nlink() == 2
            && a.uid() == unsafe { libc::geteuid() }
            && a.mode() & 0o777 == 0o600
            && target.canonicalize()? == target
            && partial.canonicalize()? == partial
        {
            std::fs::remove_file(&partial)?;
        } else {
            return Err(OmmError::invalid(
                "unknown interrupted import publication retained",
            ));
        }
    }
    if target.try_exists()? {
        validate_file(&target)?;
        if hash_file(&target)? != selection.mix_hash {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "promoted import hash mismatch",
            ));
        }
        return Ok(target);
    }
    validate_file(source)?;
    if hash_file(source)? != selection.mix_hash {
        return Err(OmmError::new(
            ErrorCode::Corrupt,
            false,
            "staged import hash mismatch",
        ));
    }
    if std::fs::symlink_metadata(&partial).is_ok() {
        validate_file(&partial)?;
        std::fs::remove_file(&partial)?;
    }
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&partial)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    drop(output);
    if hash_file(&partial)? != selection.mix_hash {
        return Err(OmmError::new(
            ErrorCode::Corrupt,
            false,
            "import copy hash mismatch",
        ));
    }
    // link is exclusive: never overwrite an unknown target created during copying.
    std::fs::hard_link(&partial, &target)?;
    std::fs::remove_file(&partial)?;
    sync_dir(dir)?;
    Ok(target)
}
fn validate_file(path: &Path) -> Result<()> {
    validate_private_file(path)?;
    let m = std::fs::symlink_metadata(path)?;
    if path.canonicalize()? != path || m.nlink() != 1 || m.mode() & 0o777 != 0o600 {
        return Err(OmmError::invalid(
            "import promotion file is not private and unique",
        ));
    }
    Ok(())
}

/// Called only after the complete cache selection has passed cleanup validation.
/// Never search DATA: this selection identifies exactly one possible promotion.
pub(super) fn abandon(paths: &Paths, owner: &ProfileOwner, selection: &Selection) -> Result<()> {
    let parent = paths.data.join("meetings/imports");
    let dir = parent.join(selection.meeting_id.to_string());
    if std::fs::symlink_metadata(&dir).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return Ok(());
    }
    Store::cleanup_unreferenced_import(paths, selection.meeting_id, &dir, || {
        for path in [&paths.data, &paths.data.join("meetings"), &parent, &dir] {
            validate_private_dir(path)?;
            if path.canonicalize()? != *path {
                return Err(OmmError::invalid(
                    "unsafe abandoned import directory retained",
                ));
            }
        }
        let marker = dir.join("import-promotion.json");
        validate_file(&marker)?;
        let expected = json!({"version":1,"profileId":owner.profile_id,"handle":selection.handle,"meetingId":selection.meeting_id,"mixHash":selection.mix_hash});
        if std::fs::metadata(&marker)?.len() > 8192
            || serde_json::from_slice::<Value>(&std::fs::read(&marker)?)? != expected
        {
            return Err(OmmError::invalid(
                "abandoned import ownership mismatch; retained",
            ));
        }
        let source = directory(paths, &selection.handle)?.join("imported-mix.wav");
        validate_file(&source)?;
        if hash_file(&source)? != selection.mix_hash {
            return Err(OmmError::invalid(
                "abandoned import source changed; retained",
            ));
        }
        let target = dir.join("imported-mix.wav");
        let partial = dir.join(".import-copy");
        let mut audio = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path == marker {
                continue;
            }
            if path != target && path != partial {
                return Err(OmmError::invalid(
                    "unknown abandoned import content retained",
                ));
            }
            audio.push(path);
        }
        let paired = audio.len() == 2;
        let source_length = std::fs::metadata(&source)?.len();
        for path in &audio {
            validate_private_file(path)?;
            let m = std::fs::symlink_metadata(path)?;
            if path.canonicalize()? != *path
                || m.mode() & 0o777 != 0o600
                || m.nlink() != if paired { 2 } else { 1 }
                || m.len() > source_length
                || (path == &target && m.len() != source_length)
            {
                return Err(OmmError::invalid(
                    "abandoned import file identity unsafe; retained",
                ));
            }
            if paired {
                let other =
                    std::fs::symlink_metadata(if path == &target { &partial } else { &target })?;
                if m.dev() != other.dev() || m.ino() != other.ino() {
                    return Err(OmmError::invalid(
                        "unknown import publication pair retained",
                    ));
                }
            }
            if path == &target {
                if hash_file(path)? != selection.mix_hash {
                    return Err(OmmError::invalid(
                        "abandoned import hash mismatch; retained",
                    ));
                }
            } else {
                // A truncated copy is known only if it is an exact prefix of the
                // still-verified source. A marker alone cannot authorize random bytes.
                let mut copy = File::open(path)?;
                let mut original = File::open(&source)?;
                let mut a = [0; 65536];
                let mut b = [0; 65536];
                loop {
                    let n = copy.read(&mut a)?;
                    if n == 0 {
                        break;
                    }
                    if original.read_exact(&mut b[..n]).is_err() || a[..n] != b[..n] {
                        return Err(OmmError::invalid(
                            "unknown interrupted import copy retained",
                        ));
                    }
                }
            }
        }
        // Validate every entry before the first unlink; marker is removed last.
        for path in audio {
            std::fs::remove_file(path)?;
        }
        std::fs::remove_file(marker)?;
        std::fs::remove_dir(&dir)?;
        sync_dir(&parent)
    })
}

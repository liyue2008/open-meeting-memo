//! Only exact non-secret registration templates can be retargeted after explicit stopped adoption.
use super::*;
use crate::core::store::hash_bytes;
const BACKUP: &str = "org.openmeetingmemo.service.pre-desktop.backup";
pub const EXTERNAL_UNLOAD_REQUIRED: &str = "external_unload_required";
pub fn require_unloaded_output(uid: u32, code: Option<i32>, stderr: &[u8]) -> Result<()> {
    let expected = format!("Could not find service \"{LABEL}\" in domain for user gui: {uid}");
    let text = std::str::from_utf8(stderr).ok();
    if uid != 0
        && stderr.len() <= 4096
        && code == Some(113)
        && text.is_some_and(|text| {
            text.trim() == expected || text.trim() == format!("Bad request.\n{expected}")
        })
    {
        return Ok(());
    }
    if code == Some(0) {
        Err(OmmError::new(
            ErrorCode::Conflict,
            false,
            EXTERNAL_UNLOAD_REQUIRED,
        ))
    } else {
        Err(OmmError::unavailable(
            "service registration state is unknown; no control operation performed",
        ))
    }
}
fn changed() -> OmmError {
    OmmError::new(
        ErrorCode::Conflict,
        false,
        "service registration changed; inspect a new recovery plan",
    )
}
pub fn inspect(
    home: &Path,
    paths: &Paths,
    runtime: &crate::core::runtime::RuntimeLocation,
    target: &Path,
) -> Result<(String, Vec<u8>, String)> {
    validate_executable(target)?;
    for parent in [
        home.to_path_buf(),
        home.join("Library"),
        home.join("Library/LaunchAgents"),
    ] {
        let meta = std::fs::symlink_metadata(&parent)?;
        if !meta.is_dir()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o022 != 0
            || parent.canonicalize()? != parent
        {
            return Err(changed());
        }
    }
    let path = plist(home);
    validate_private_file(&path)?;
    let bytes = std::fs::read(&path)?;
    if bytes.len() > 64 * 1024 {
        return Err(changed());
    }
    let profile = decode_profile(&bytes)?;
    profile.check_roots(paths)?;
    profile.check_runtime(runtime)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| changed())?;
    let encoded = text
        .split("<key>ProgramArguments</key><array><string>")
        .nth(1)
        .and_then(|s| s.split("</string>").next())
        .ok_or_else(changed)?;
    let executable = encoded
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    let executable = Path::new(&executable);
    validate_executable(executable)?;
    let kind = ["Standard", "Background"]
        .into_iter()
        .find(|kind| render(home, executable, &profile, kind).is_ok_and(|v| v.as_bytes() == bytes))
        .ok_or_else(changed)?;
    let meta = std::fs::symlink_metadata(executable)?;
    let source_identity = hash_bytes(&serde_json::to_vec(&(
        executable,
        meta.dev(),
        meta.ino(),
        meta.uid(),
        meta.mode(),
        crate::core::store::hash_file(executable)?,
    ))?);
    Ok((
        hash_bytes(&bytes),
        render(home, target, &profile, kind)?.into_bytes(),
        source_identity,
    ))
}
pub struct Retarget {
    directory: Directory,
    old: Snapshot,
    replacement: Vec<u8>,
    published: Option<Snapshot>,
}
impl Retarget {
    pub fn lock(
        home: &Path,
        paths: &Paths,
        runtime: &crate::core::runtime::RuntimeLocation,
        target: &Path,
        expected: &str,
    ) -> Result<Self> {
        let directory = Directory::open(home)?;
        let (hash, replacement, _) = inspect(home, paths, runtime, target)?;
        if hash != expected {
            return Err(changed());
        }
        let old = directory.snapshot(PLIST)?.ok_or_else(changed)?;
        if hash_bytes(&old.bytes) != hash {
            return Err(changed());
        }
        Ok(Self {
            directory,
            old,
            replacement,
            published: None,
        })
    }
    pub async fn publish(&mut self, runner: &dyn ServiceRunner) -> Result<()> {
        runner.require_unloaded().await?;
        self.directory.verify(PLIST, Some(&self.old))?;
        match self.directory.snapshot(BACKUP)? {
            Some(b) if b.bytes == self.old.bytes => (),
            Some(_) => return Err(changed()),
            None => {
                self.directory.publish(BACKUP, None, &self.old.bytes)?;
            }
        }
        // Never control any loaded legacy job. Recheck immediately before CAS publication.
        runner.require_unloaded().await?;
        self.published=Some(self.directory.publish(PLIST,Some(&self.old),&self.replacement).map_err(|_|OmmError::new(ErrorCode::Conflict,false,"service job unloaded but registration changed; files retained; review registration before retry"))?);
        Ok(())
    }
    pub fn rollback(&self) -> Result<()> {
        if let Some(published) = &self.published {
            self.directory
                .publish(PLIST, Some(published), &self.old.bytes)?;
        }
        Ok(())
    }
}

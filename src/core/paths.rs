use super::contracts::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub warnings: Vec<String>,
}
impl Paths {
    pub fn resolve(home: &Path, env: &BTreeMap<String, String>) -> Result<Self> {
        if !home.is_absolute() {
            return Err(OmmError::invalid("HOME must be absolute"));
        }
        let mut warnings = vec![];
        let mut root = |key: &str, fallback: &str| {
            let value = env.get(key).filter(|v| !v.is_empty());
            match value {
                Some(v) if Path::new(v).is_absolute() => PathBuf::from(v).join("omm"),
                Some(_) => {
                    warnings.push(format!("ignored relative {key}"));
                    home.join(fallback).join("omm")
                }
                None => home.join(fallback).join("omm"),
            }
        };
        Ok(Self {
            config: root("XDG_CONFIG_HOME", ".config"),
            data: root("XDG_DATA_HOME", ".local/share"),
            state: root("XDG_STATE_HOME", ".local/state"),
            cache: root("XDG_CACHE_HOME", ".cache"),
            warnings,
        })
    }
    pub fn from_env() -> Result<Self> {
        let env = std::env::vars().collect();
        let home = std::env::var_os("HOME").ok_or_else(|| OmmError::invalid("HOME is missing"))?;
        Self::resolve(Path::new(&home), &env)
    }
    pub fn create_private_dirs(&self) -> Result<()> {
        for p in [&self.config, &self.data, &self.state, &self.cache] {
            private_dir(p)?;
        }
        Ok(())
    }
    pub fn runtime_location(
        &self,
        env: &BTreeMap<String, String>,
    ) -> Result<super::runtime::RuntimeLocation> {
        super::runtime::RuntimeLocation::resolve(env, super::runtime::system_user_temp)
    }
    pub fn runtime(&self, env: &BTreeMap<String, String>) -> Result<PathBuf> {
        Ok(self.runtime_location(env)?.path)
    }
}
pub fn validate_private_dir(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o777 != 0o700 {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "private directory must be owned by current user with mode 0700",
        ));
    }
    Ok(())
}
pub fn private_dir(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(OmmError::invalid("private path must be absolute"));
    }
    if fs::symlink_metadata(path).is_ok() {
        return validate_private_dir(path);
    }
    // DirBuilder's mode applies only to newly created directories, never existing XDG roots.
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    validate_private_dir(path)
}
pub fn controlled_file(root: &Path, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(OmmError::invalid(
            "artifact path must be absolute without traversal",
        ));
    }
    let canonical = path.canonicalize()?;
    if canonical != path
        || !canonical.starts_with(root.canonicalize()?)
        || !fs::symlink_metadata(path)?.is_file()
    {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "file is outside controlled root or symlinked",
        ));
    }
    Ok(canonical)
}
pub fn write_private_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
pub fn validate_private_file(path: &Path) -> Result<()> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "private file ownership or permissions unsafe",
        ));
    }
    Ok(())
}
pub fn write_private_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        validate_private_file(path)?;
    }
    let parent = path
        .parent()
        .ok_or_else(|| OmmError::invalid("missing file parent"))?;
    let tmp = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    write_private_new(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    sync_dir(parent)
}
pub fn sync_dir(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

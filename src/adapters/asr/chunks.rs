//! Persistent, input/configuration-bound speech checkpoints; final artifacts belong to Store.
use crate::core::{
    contracts::*,
    paths::{controlled_file, private_dir, validate_private_dir},
    store::hash_bytes,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
#[derive(Serialize, Deserialize)]
struct Chunk {
    version: u32,
    start: usize,
    end: usize,
    text: String,
    sha256: String,
}
#[cfg(test)]
pub(super) fn directory(root: &Path, input: &Path, config: &AsrConfig, source: AudioSource) -> Result<PathBuf> {
    directory_with_source(root,input,config,source,&crate::core::store::hash_file(input)?)
}
pub(super) fn directory_with_source(
    root: &Path,
    input: &Path,
    config: &AsrConfig,
    source: AudioSource,
    source_hash: &str,
) -> Result<PathBuf> {
    validate_private_dir(root)?;
    if root.canonicalize()? != root {
        return Err(OmmError::invalid("chunk workspace path is not canonical"));
    }
    let key = hash_bytes(&serde_json::to_vec(&(
        2u32, // Decoder window/padding segmentation version.
        crate::core::store::hash_file(input)?,
        source_hash,
        config,
        source,
    ))?);
    let dir = root.join(key);
    private_dir(&dir)?;
    Ok(dir)
}
fn path(root: &Path, start: usize, end: usize) -> PathBuf {
    root.join(format!("{start}-{end}.json"))
}
pub(super) fn read(root: &Path, start: usize, end: usize) -> Result<Option<String>> {
    let p = path(root, start, end);
    if !p.exists() {
        if fs::symlink_metadata(&p).is_ok() {
            return Err(OmmError::invalid("chunk checkpoint path is invalid"));
        }
        return Ok(None);
    }
    controlled_file(root, &p)?;
    if fs::metadata(&p)?.len() > 1024 * 1024 + 1024 {
        return Ok(None);
    }
    let Ok(c) = serde_json::from_slice::<Chunk>(&fs::read(p)?) else {
        return Ok(None);
    };
    if c.version != 2
        || c.start != start
        || c.end != end
        || c.text.len() > 1024 * 1024
        || hash_bytes(c.text.as_bytes()) != c.sha256
    {
        return Ok(None);
    }
    Ok(Some(c.text))
}
pub(super) fn write(root: &Path, start: usize, end: usize, text: &str) -> Result<()> {
    let dest = path(root, start, end);
    if fs::symlink_metadata(&dest).is_ok() {
        controlled_file(root, &dest)?;
    }
    let temp = root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let c = Chunk {
        version: 2,
        start,
        end,
        text: text.into(),
        sha256: hash_bytes(text.as_bytes()),
    };
    f.write_all(&serde_json::to_vec(&c)?)?;
    f.sync_all()?;
    fs::rename(temp, dest)?;
    fs::File::open(root)?.sync_all()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_cache_is_bound_to_input_config_source_and_corruption_is_not_a_checkpoint() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        private_dir(&root).unwrap();
        let work = root.join("lease");
        private_dir(&work).unwrap();
        let input = work.join("float");
        fs::write(&input, b"generated samples").unwrap();
        let cfg = AsrConfig::default();
        let a = directory(&work, &input, &cfg, AudioSource::Mic).unwrap();
        write(&a, 0, 16000, "generated").unwrap();
        assert_eq!(read(&a, 0, 16000).unwrap(), Some("generated".into()));
        let b = directory(&work, &input, &cfg, AudioSource::System).unwrap();
        assert_ne!(a, b);
        assert!(read(&b, 0, 16000).unwrap().is_none());
        let mut new = cfg.clone();
        new.model_version = "new".into();
        assert_ne!(a, directory(&work, &input, &new, AudioSource::Mic).unwrap());
        let p = path(&a, 0, 16000);
        fs::write(&p, b"{broken").unwrap();
        assert!(read(&a, 0, 16000).unwrap().is_none());
        std::fs::remove_file(&p).unwrap();
        std::os::unix::fs::symlink(&input, p).unwrap();
        assert!(read(&a, 0, 16000).is_err());
    }
}

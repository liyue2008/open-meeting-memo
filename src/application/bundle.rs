//! The application only starts its own hash-verified CLI and dependencies, never a renderer path.
use crate::core::{contracts::*, store::hash_file};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub version: String,
    pub source_revision: String,
    pub files: BTreeMap<String, String>,
}
#[derive(Clone)]
pub struct Bundle {
    pub resources: PathBuf,
    pub manifest: Manifest,
    pub manifest_hash: String,
}
impl Bundle {
    pub fn locate() -> Result<Self> {
        let exe = std::env::current_exe()?.canonicalize()?;
        let contents = exe
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| OmmError::unavailable("application bundle unavailable"))?;
        Self::open(&contents.join("Resources"))
    }
    pub fn open(resources: &Path) -> Result<Self> {
        let root = resources.canonicalize()?;
        let path = root.join("bundle-manifest.json");
        let metadata = std::fs::symlink_metadata(&path)?;
        if !metadata.is_file()
            || metadata.len() > 1024 * 1024
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
        {
            return Err(OmmError::invalid("unsafe bundle manifest"));
        }
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(&path)?)?;
        if manifest.schema_version != 1
            || manifest.version != env!("CARGO_PKG_VERSION")
            || manifest.source_revision.len() != 40
            || manifest.files.len() > 4096
        {
            return Err(OmmError::invalid("incompatible bundle manifest"));
        }
        let bundle = Self {
            resources: root,
            manifest,
            manifest_hash: hash_file(&path)?,
        };
        bundle.verify()?;
        Ok(bundle)
    }
    pub fn file(&self, name: &str) -> Result<PathBuf> {
        let expected = self
            .manifest
            .files
            .get(name)
            .ok_or_else(|| OmmError::unavailable("required bundled dependency missing"))?;
        let relative = Path::new(name);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(OmmError::invalid("invalid bundle resource"));
        }
        let path = self.resources.join(relative);
        let canonical = path.canonicalize()?;
        if path != canonical || !canonical.starts_with(&self.resources) {
            return Err(OmmError::invalid("bundle resource escaped trusted root"));
        }
        for parent in path
            .ancestors()
            .take_while(|p| p.starts_with(&self.resources))
        {
            let m = std::fs::symlink_metadata(parent)?;
            if m.uid() != unsafe { libc::geteuid() }
                || m.mode() & 0o022 != 0
                || m.file_type().is_symlink()
            {
                return Err(OmmError::invalid("unsafe bundle resource ownership"));
            }
        }
        if hash_file(&path)? != *expected {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "bundle resource hash mismatch",
            ));
        }
        Ok(path)
    }
    pub fn verify(&self) -> Result<()> {
        for name in self.manifest.files.keys() {
            self.file(name)?;
        }
        for name in [
            "bin/omm",
            "bin/omm-asr-worker",
            "OMMCapture.app/Contents/MacOS/omm-capture",
            "sherpa/libsherpa-onnx-c-api.dylib",
            "audio-toolchain/manifest.json",
        ] {
            self.file(name)?;
        }
        Ok(())
    }
    pub fn cli(&self) -> Result<PathBuf> {
        self.file("bin/omm")
    }
}

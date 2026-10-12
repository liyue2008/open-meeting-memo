use super::{contracts::*, paths::*, store::hash_file};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Component, Path},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub kind: String,
    pub url: String,
    pub sha256: String,
    pub archive_bytes: u64,
    pub installed_bytes: u64,
    pub files: BTreeMap<String, String>,
}
#[derive(Deserialize)]
struct Manifest {
    assets: Vec<Asset>,
}
pub fn assets() -> Result<Vec<Asset>> {
    Ok(serde_json::from_str::<Manifest>(include_str!("../../packaging/asr/models.json"))?.assets)
}
pub fn asset(id: &str) -> Result<Asset> {
    assets()?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| OmmError::invalid("unknown model id"))
}
pub fn verify(paths: &Paths, asset: &Asset) -> Result<()> {
    let root = paths.data.join("models").join(&asset.id);
    for (name, hash) in &asset.files {
        let file = root.join(name);
        if !file.is_file() {
            return Err(OmmError::unavailable(
                "model file missing; download is explicit",
            ));
        }
        controlled_file(&root, &file)?;
        if hash_file(&file)? != *hash {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "installed model SHA256 mismatch",
            ));
        }
    }
    Ok(())
}
/// Install only hash-pinned files from a fully verified archive. Tests use a private small manifest.
pub fn install_archive(paths: &Paths, asset: &Asset, archive: &Path) -> Result<()> {
    if asset.id.is_empty()
        || asset.id.contains('/')
        || asset.id.contains("..")
        || asset.files.is_empty()
        || asset
            .files
            .keys()
            .any(|n| n.contains('/') || n.contains(".."))
    {
        return Err(OmmError::invalid("invalid model manifest"));
    }
    if fs::metadata(archive)?.len() != asset.archive_bytes || hash_file(archive)? != asset.sha256 {
        return Err(OmmError::new(
            ErrorCode::Corrupt,
            false,
            "download size or SHA256 mismatch",
        ));
    }
    let root = paths.data.join("models");
    private_dir(&root)?;
    let final_dir = root.join(&asset.id);
    if final_dir.exists() {
        return verify(paths, asset);
    }
    let staging = root.join(format!(".install-{}", uuid::Uuid::new_v4()));
    private_dir(&staging)?;
    let result = (|| {
        if asset.kind == "file" {
            if asset.files.len() != 1 {
                return Err(OmmError::invalid("file model manifest invalid"));
            }
            let dest = staging.join(asset.files.keys().next().unwrap());
            let mut source = fs::File::open(archive)?;
            use std::os::unix::fs::OpenOptionsExt;
            let mut out = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(dest)?;
            std::io::copy(&mut source, &mut out)?;
            out.sync_all()?;
        } else if asset.kind == "tarbz2" {
            let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(fs::File::open(archive)?));
            let mut expanded = 0u64;
            for entry in tar.entries()? {
                let mut entry = entry?;
                let path = entry.path()?.into_owned();
                if path.is_absolute()
                    || path
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                    || !(entry.header().entry_type().is_file()
                        || entry.header().entry_type().is_dir())
                {
                    return Err(OmmError::new(
                        ErrorCode::Corrupt,
                        false,
                        "model archive contains unsafe entry",
                    ));
                }
                expanded = expanded
                    .checked_add(entry.size())
                    .filter(|n| *n <= asset.archive_bytes.saturating_mul(10))
                    .ok_or_else(|| {
                        OmmError::new(
                            ErrorCode::Corrupt,
                            false,
                            "model archive expands beyond limit",
                        )
                    })?;
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if entry.header().entry_type().is_file() && asset.files.contains_key(name) {
                    if entry.size() > asset.installed_bytes {
                        return Err(OmmError::new(
                            ErrorCode::Corrupt,
                            false,
                            "model entry exceeds installed budget",
                        ));
                    }
                    use std::os::unix::fs::OpenOptionsExt;
                    let mut out = fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .mode(0o600)
                        .open(staging.join(name))?;
                    std::io::copy(&mut entry, &mut out)?;
                    out.sync_all()?;
                }
            }
        } else {
            return Err(OmmError::invalid("unsupported model archive"));
        }
        for (name, expected) in &asset.files {
            if !staging.join(name).is_file() || hash_file(&staging.join(name))? != *expected {
                return Err(OmmError::new(
                    ErrorCode::Corrupt,
                    false,
                    "extracted model SHA256 mismatch",
                ));
            }
        }
        sync_dir(&staging)?;
        fs::rename(&staging, &final_dir)?;
        sync_dir(&root)
    })();
    if staging.exists() {
        let _ = fs::remove_dir_all(staging);
    }
    result
}
#[async_trait::async_trait]
pub trait DownloadSource: Send {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>>;
}
struct NetworkSource(reqwest::Response);
#[async_trait::async_trait]
impl DownloadSource for NetworkSource {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        self.0
            .chunk()
            .await
            .map(|b| b.map(|b| b.to_vec()))
            .map_err(|_| OmmError::new(ErrorCode::Backend, true, "model download interrupted"))
    }
}
pub async fn download(paths: &Paths, asset: &Asset) -> Result<()> {
    if !asset.url.starts_with("https://") {
        return Err(OmmError::invalid("compiled download URL must use HTTPS"));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(7200))
        .build()
        .map_err(|_| OmmError::unavailable("download client unavailable"))?;
    let response = client
        .get(&asset.url)
        .send()
        .await
        .map_err(|_| OmmError::new(ErrorCode::Backend, true, "model download failed"))?
        .error_for_status()
        .map_err(|_| OmmError::new(ErrorCode::Backend, true, "model server rejected download"))?;
    download_from_source(paths, asset, &mut NetworkSource(response)).await
}
/// Injectable transport seam, not a CLI manifest/URL override.
pub async fn download_from_source(
    paths: &Paths,
    asset: &Asset,
    source: &mut dyn DownloadSource,
) -> Result<()> {
    if !asset.url.starts_with("https://") {
        return Err(OmmError::invalid("compiled download URL must use HTTPS"));
    }
    private_dir(&paths.cache.join("downloads"))?;
    let file = paths
        .cache
        .join("downloads")
        .join(uuid::Uuid::new_v4().to_string());
    write_private_new(&file, &[])?;
    let result = async {
        let mut out = fs::OpenOptions::new().write(true).open(&file)?;
        let mut total = 0u64;
        while let Some(chunk) = source.next_chunk().await? {
            total = total
                .checked_add(chunk.len() as u64)
                .filter(|n| *n <= asset.archive_bytes)
                .ok_or_else(|| {
                    OmmError::new(
                        ErrorCode::Corrupt,
                        false,
                        "model download exceeds manifest size",
                    )
                })?;
            out.write_all(&chunk)?;
        }
        out.sync_all()?;
        install_archive(paths, asset, &file)
    }
    .await;
    let _ = fs::remove_file(file);
    result
}
pub fn remove(paths: &Paths, asset: &Asset) -> Result<()> {
    let dir = paths.data.join("models").join(&asset.id);
    if dir.exists() {
        validate_private_dir(&dir)?;
        if dir.canonicalize()? != dir {
            return Err(OmmError::invalid("model directory is symlinked"));
        }
        fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn mutation_lock(paths: &Paths) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = paths.state.join("models.lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&file)?;
    crate::core::paths::validate_private_file(&file)?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| {
        OmmError::new(
            ErrorCode::Conflict,
            false,
            "model operation already running",
        )
    })?;
    Ok(lock)
}

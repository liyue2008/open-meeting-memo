//! Explicit user-only installation of the source-built, pinned audio toolchain.
use super::{
    contracts::*,
    paths::{private_dir, write_private_new, Paths},
    store::{hash_bytes, hash_file, Store},
};
use crate::adapters::asr::codec::AudioCodec;
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};
const SOURCES: &str = include_str!("../../packaging/audio/sources.json");
pub fn validate_manifest(value: &Value) -> Result<()> {
    let fields = [
        "schemaVersion",
        "ffmpegVersion",
        "opusVersion",
        "license",
        "gplEnabled",
        "networkEnabled",
        "sourceManifestSha256",
        "ffmpegSha256",
        "ffprobeSha256",
        "signing",
    ];
    if !value
        .as_object()
        .is_some_and(|o| o.len() == fields.len() && o.keys().all(|k| fields.contains(&k.as_str())))
        || value["signing"] != "ad-hoc-development"
    {
        return Err(OmmError::invalid(
            "audio toolchain manifest contains unknown metadata",
        ));
    }
    if !["ffmpegSha256", "ffprobeSha256"].iter().all(|k| {
        value[k]
            .as_str()
            .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    }) {
        return Err(OmmError::invalid("audio toolchain hashes are invalid"));
    }
    if value["schemaVersion"] != 1
        || value["ffmpegVersion"] != "8.1.2"
        || value["opusVersion"] != "1.5.2"
        || value["license"] != "LGPL-2.1-or-later"
        || value["gplEnabled"] != false
        || value["networkEnabled"] != false
        || value["sourceManifestSha256"] != hash_bytes(SOURCES.as_bytes())
    {
        return Err(OmmError::invalid(
            "audio toolchain manifest is not the pinned LGPL build",
        ));
    }
    Ok(())
}
pub fn check(p: &Paths) -> Value {
    let present = p.data.join("audio-toolchain/manifest.json").exists();
    let ready = AudioCodec::from_paths(p).is_ok();
    json!({"configured":present,"ready":ready,"ffmpegVersion":"8.1.2","opusVersion":"1.5.2","format":"ogg/opus","sampleRate":48000,"channels":1})
}
fn files() -> [&'static str; 8] {
    [
        "ffmpeg",
        "ffprobe",
        "manifest.json",
        "sources.json",
        "build-dev.sh",
        "licenses/ffmpeg.txt",
        "licenses/opus.txt",
        "sources/ffmpeg-8.1.2.tar.xz",
    ]
}
fn verify_bundle(root: &Path) -> Result<Vec<&'static str>> {
    let mut names = files().to_vec();
    names.push("sources/opus-1.5.2.tar.gz");
    for n in &names {
        let path = root.join(n);
        let metadata = fs::symlink_metadata(&path)?;
        use std::os::unix::fs::MetadataExt;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
            || metadata.len() > 64 * 1024 * 1024
            || path.canonicalize()? != path
        {
            return Err(OmmError::invalid(
                "audio toolchain bundle contains unsafe files",
            ));
        }
        for parent in path.ancestors().skip(1).take_while(|p| p.starts_with(root)) {
            let m = fs::symlink_metadata(parent)?;
            if !m.is_dir() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o022 != 0 {
                return Err(OmmError::invalid("audio toolchain bundle parent is unsafe"));
            }
        }
    }
    let metadata = fs::symlink_metadata(root)?;
    use std::os::unix::fs::MetadataExt;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err(OmmError::invalid(
            "audio toolchain bundle directory is unsafe",
        ));
    }
    AudioCodec::from_manifest(&root.join("manifest.json"))?;
    if fs::read(root.join("sources.json"))? != SOURCES.as_bytes() {
        return Err(OmmError::invalid(
            "audio toolchain source manifest differs from pinned sources",
        ));
    }
    let pins: Value = serde_json::from_str(SOURCES)?;
    for (name, archive) in [
        ("ffmpeg", "ffmpeg-8.1.2.tar.xz"),
        ("opus", "opus-1.5.2.tar.gz"),
    ] {
        if hash_file(&root.join("sources").join(archive))? != pins[name]["sha256"].as_str().unwrap()
        {
            return Err(OmmError::new(
                ErrorCode::Corrupt,
                false,
                "audio toolchain source checksum mismatch",
            ));
        }
    }
    let manifest: Value = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    if hash_file(&root.join("ffprobe"))? != manifest["ffprobeSha256"].as_str().unwrap_or("") {
        return Err(OmmError::new(
            ErrorCode::Corrupt,
            false,
            "audio inspection tool checksum mismatch",
        ));
    }
    for (name, expected) in [
        (
            "licenses/ffmpeg.txt",
            include_str!("../../packaging/audio/licenses/ffmpeg-LGPL-2.1.txt"),
        ),
        (
            "licenses/opus.txt",
            include_str!("../../packaging/audio/licenses/opus-BSD-3-Clause.txt"),
        ),
        (
            "build-dev.sh",
            include_str!("../../packaging/audio/build-dev.sh"),
        ),
    ] {
        if fs::read(root.join(name))? != expected.as_bytes() {
            return Err(OmmError::invalid(
                "audio toolchain license or build script differs from pinned source",
            ));
        }
    }
    Ok(names)
}
pub fn install(p: &Paths, from: &Path) -> Result<Value> {
    if unsafe { libc::geteuid() } == 0 {
        return Err(OmmError::new(
            ErrorCode::Permission,
            false,
            "audio toolchain installation is user-only",
        ));
    }
    let root = from.canonicalize()?;
    if root != from || !from.is_absolute() {
        return Err(OmmError::invalid(
            "audio toolchain source must be a canonical absolute directory",
        ));
    }
    let names = verify_bundle(&root)?;
    p.create_private_dirs()?;
    let lock_path = p.state.join("audio-readers.lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&lock_path)?;
    super::paths::validate_private_file(&lock_path)?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| {
        OmmError::new(
            ErrorCode::Conflict,
            true,
            "stop audio work before installing the toolchain",
        )
    })?;
    if p.state.join("state.sqlite").exists() {
        let store = Store::open_read_only(p)?;
        if store
            .state::<Value>("recording")?
            .is_some_and(|v| !v.is_null())
        {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                true,
                "seal recording before installing the toolchain",
            ));
        }
    }
    let id = uuid::Uuid::new_v4();
    let destination = p.data.join("audio-toolchain");
    let stage = p.data.join(format!(".audio-toolchain-{id}.stage"));
    private_dir(&stage)?;
    let stage = stage.canonicalize()?;
    let result = (|| -> Result<Value> {
        for name in names {
            let source = root.join(name);
            let output = stage.join(name);
            private_dir(output.parent().unwrap())?;
            write_private_new(&output, &fs::read(&source)?)?;
            if name == "ffmpeg" || name == "ffprobe" || name == "build-dev.sh" {
                fs::set_permissions(&output, fs::Permissions::from_mode(0o700))?;
            }
        }
        verify_bundle(&stage)?;
        for sub in ["sources", "licenses", ""] {
            fs::File::open(stage.join(sub))?.sync_all()?;
        }
        let old = p.data.join(format!(".audio-toolchain-{id}.previous"));
        if destination.exists() {
            super::paths::validate_private_dir(&destination)?;
            let owned_root = destination.canonicalize()?;
            let manifest = owned_root.join("manifest.json");
            super::paths::controlled_file(&owned_root, &manifest)?;
            AudioCodec::from_manifest(&manifest)?;
            let backup = p.state.join("audio-toolchain-backups").join(id.to_string());
            private_dir(&backup)?;
            write_private_new(
                &backup.join("manifest.json"),
                &fs::read(destination.join("manifest.json"))?,
            )?;
            fs::File::open(&backup)?.sync_all()?;
            fs::rename(&destination, &old)?;
        }
        if let Err(error) = fs::rename(&stage, &destination) {
            if old.exists() {
                fs::rename(&old, &destination)?;
            }
            return Err(error.into());
        }
        fs::File::open(&p.data)?.sync_all()?;
        // Previous non-secret dependencies remain recoverable; no recording/configuration is moved.
        Ok(
            json!({"installed":true,"verified":true,"restartServiceRequired":true,"path":destination,"previous":old.exists().then_some(old)}),
        )
    })();
    if stage.exists() {
        fs::remove_dir_all(stage)?;
    }
    result
}

//! Explicit two-step recovery: adopt a proven stopped profile, then separately start the bundle.
use super::{bundle::Bundle, profile};
use crate::{
    core::{
        config::AppConfig,
        contracts::*,
        paths::*,
        runtime::{self, RuntimeGuard, RuntimeLocation},
        store::{hash_bytes, hash_file},
    },
    service::registration::{self, ServiceRunner},
};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
#[derive(Clone)]
pub struct RecoverySnapshot {
    pub fingerprint: String,
    pub registration_hash: String,
    pub expires: i64,
}
fn refused() -> OmmError {
    OmmError::new(ErrorCode::Conflict,false,"stopped legacy recovery cannot be verified; retain the profile and review service registration, workspaces and SQLite sidecars")
}
fn file_fingerprint(path: &Path) -> Result<Value> {
    validate_private_file(path)?;
    let m = std::fs::symlink_metadata(path)?;
    if m.nlink() != 1 {
        return Err(refused());
    }
    Ok(json!([
        m.dev(),
        m.ino(),
        m.uid(),
        m.mode(),
        hash_file(path)?
    ]))
}
/// Probe existing lock inodes only: planning must not create profile files.
pub fn existing_lock(path: &Path) -> Result<File> {
    validate_private_file(path)?;
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let a = f.metadata()?;
    let b = std::fs::symlink_metadata(path)?;
    if a.dev() != b.dev() || a.ino() != b.ino() || a.nlink() != 1 {
        return Err(refused());
    }
    fs2::FileExt::try_lock_exclusive(&f).map_err(|_| refused())?;
    Ok(f)
}
fn stopped(paths: &Paths, runtime: &RuntimeLocation, owned: bool) -> Result<()> {
    for root in [
        &paths.config,
        &paths.data,
        &paths.state,
        &paths.cache,
        &runtime.path,
    ] {
        validate_private_dir(root)?;
        if root.canonicalize()? != *root {
            return Err(refused());
        }
    }
    if runtime.path.join("owner.json").try_exists()? {
        let owner = runtime::read_owner(&runtime.path, &paths.state)?;
        if runtime::owner_is_live(&owner) && !(owned && owner.pid == std::process::id() as i32) {
            return Err(refused());
        }
    } else if runtime.path.join("service.sock").try_exists()? {
        return Err(refused());
    }
    let capture = runtime.path.join("capture");
    if capture.try_exists()? {
        validate_private_dir(&capture)?;
        if std::fs::read_dir(capture)?.next().is_some() {
            return Err(refused());
        }
    }
    for name in [
        "pi",
        "segment-tasks",
        "archive-tasks",
        "tasks",
        "capture",
        "desktop-imports",
        "downloads",
    ] {
        let p = paths.cache.join(name);
        if p.try_exists()? {
            validate_private_dir(&p)?;
            if std::fs::read_dir(p)?.next().is_some() {
                return Err(refused());
            }
        }
    }
    Ok(())
}
pub(crate) fn database(paths: &Paths) -> Result<Value> {
    let db = paths.state.join("state.sqlite");
    let before = file_fingerprint(&db)?;
    // Immutable read is permitted ONLY when every SQLite sidecar is absent. Never ignore WAL.
    for suffix in ["-wal", "-shm", "-journal"] {
        if paths
            .state
            .join(format!("state.sqlite{suffix}"))
            .try_exists()?
        {
            return Err(refused());
        }
    }
    let encoded = db
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    let conn = Connection::open_with_flags(
        format!("file:{encoded}?mode=ro&immutable=1"),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let schema_refused = || {
        OmmError::new(ErrorCode::Conflict, false, "legacy SQLite schema is unsupported; preserve it and review with the matching application version")
    };
    if conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? != 0 {
        return Err(schema_refused());
    }
    let expected: BTreeMap<String, String> =
        serde_json::from_str(include_str!("recovery-schema.json"))?;
    let mut tables = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT name,sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%'",
        )?;
        for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
            let (name, sql) = row?;
            if expected.get(&name) != Some(&sql) {
                return Err(schema_refused());
            }
            tables.insert(name, sql);
        }
    }
    // Historical measurement setup runs two ordered CREATEs before its transaction.
    // First-only is a possible interrupted setup; second-only is not a known schema.
    if tables.contains_key("asr_measurement_totals") && !tables.contains_key("asr_measurements") {
        return Err(schema_refused());
    }
    for required in [
        "settings",
        "meetings",
        "tasks",
        "commits",
        "retries",
        "segmented_sessions",
        "segment_jobs",
        "segment_archives",
    ] {
        if !tables.contains_key(required) {
            return Err(schema_refused());
        }
    }
    let busy:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE lease IS NOT NULL OR status IN ('transcribing','summarizing')) OR EXISTS(SELECT 1 FROM segment_jobs WHERE lease IS NOT NULL OR status='running') OR EXISTS(SELECT 1 FROM segment_archives WHERE lease IS NOT NULL OR status='running') OR EXISTS(SELECT 1 FROM segmented_sessions WHERE sealed!=1 OR typeof(sealed)!='integer') OR EXISTS(SELECT 1 FROM commits WHERE done!=1)",[],|r|r.get(0))?;
    if busy {
        return Err(refused());
    }
    let mut stmt=conn.prepare("SELECT key,document FROM settings WHERE key IN ('recording','pending_stop','capture_helper_expected','capture_error')")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (key, document) = row?;
        let v: Value = serde_json::from_str(&document)?;
        if !(key == "capture_helper_expected" && v == false || key == "pending_stop" && v.is_null())
        {
            return Err(refused());
        }
    }
    if let Ok(file) = existing_lock(&paths.state.join("audio-readers.lock")) {
        drop(file);
    } else if paths.state.join("audio-readers.lock").try_exists()? {
        return Err(refused());
    }
    if before != file_fingerprint(&db)? {
        return Err(refused());
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        if paths
            .state
            .join(format!("state.sqlite{suffix}"))
            .try_exists()?
        {
            return Err(refused());
        }
    }
    Ok(before)
}
fn snapshot(
    paths: &Paths,
    home: &Path,
    runtime: &RuntimeLocation,
    bundle: &Bundle,
    owned: bool,
) -> Result<(String, String)> {
    bundle.verify()?;
    if profile::read(paths)?.is_some() {
        return Err(refused());
    }
    stopped(paths, runtime, owned)?;
    let cfg = AppConfig::load(paths)?;
    cfg.validate_policy()?;
    let (registration_hash, _, source_identity) =
        registration::recovery::inspect(home, paths, runtime, &bundle.cli()?)?;
    let value = json!({"profile":profile::profile_id(paths)?,"bundle":bundle.manifest_hash,"registration":registration_hash,"sourceIdentity":source_identity,"config":file_fingerprint(&paths.config.join("config.json"))?,"database":database(paths)?});
    Ok((hash_bytes(&serde_json::to_vec(&value)?), registration_hash))
}
pub(crate) fn start_snapshot(
    paths: &Paths,
    home: &Path,
    runtime: &RuntimeLocation,
    bundle: &Bundle,
    owned: bool,
) -> Result<String> {
    stopped(paths, runtime, owned)?;
    AppConfig::load(paths)?.validate_policy()?;
    let registration = if home
        .join("Library/LaunchAgents/org.openmeetingmemo.service.plist")
        .try_exists()?
    {
        let (hash, _, source) =
            registration::recovery::inspect(home, paths, runtime, &bundle.cli()?)?;
        Some(json!([hash, source]))
    } else {
        None
    };
    let database = if paths.state.join("state.sqlite").try_exists()? {
        Some(database(paths)?)
    } else {
        let meetings = paths.data.join("meetings");
        if meetings.try_exists()? && std::fs::read_dir(meetings)?.next().is_some() {
            return Err(refused());
        }
        None
    };
    Ok(hash_bytes(&serde_json::to_vec(
        &json!({"owner":profile::read(paths)?,"profile":profile::profile_id(paths)?,"config":file_fingerprint(&paths.config.join("config.json"))?,"database":database,"registration":registration,"bundle":bundle.manifest_hash}),
    )?))
}
pub(crate) fn stopped_start_snapshot(
    paths: &Paths,
    home: &Path,
    runtime: &RuntimeLocation,
    bundle: &Bundle,
) -> Result<String> {
    let _runtime = existing_lock(&runtime.path.join("service.lock"))?;
    let _state = existing_lock(&paths.state.join("service.lock"))?;
    start_snapshot(paths, home, runtime, bundle, false)
}
pub fn inspect(
    paths: &Paths,
    home: &Path,
    runtime: &RuntimeLocation,
    bundle: &Bundle,
) -> Result<RecoverySnapshot> {
    let _runtime = existing_lock(&runtime.path.join("service.lock"))?;
    let _state = existing_lock(&paths.state.join("service.lock"))?;
    let (fingerprint, registration_hash) = snapshot(paths, home, runtime, bundle, false)?;
    Ok(RecoverySnapshot {
        fingerprint,
        registration_hash,
        expires: chrono::Utc::now().timestamp_millis() + 60000,
    })
}
pub async fn adopt(
    paths: &Paths,
    home: &Path,
    runtime: &RuntimeLocation,
    bundle: &Bundle,
    plan: &RecoverySnapshot,
    runner: &dyn ServiceRunner,
) -> Result<()> {
    if plan.expires < chrono::Utc::now().timestamp_millis() {
        return Err(refused());
    }
    // Same nonblocking order for all recovery operations: lifecycle (caller), registration, runtime/state, writers.
    let mut registration = registration::recovery::Retarget::lock(
        home,
        paths,
        runtime,
        &bundle.cli()?,
        &plan.registration_hash,
    )?;
    let guard = RuntimeGuard::acquire(runtime, &paths.state)?;
    let writer = profile::writer_lock(paths)?;
    let _config_writer = crate::core::config_transaction::ConfigTransaction::acquire(paths)?;
    let (fingerprint, _) = snapshot(paths, home, runtime, bundle, true)?;
    if fingerprint != plan.fingerprint {
        return Err(refused());
    }
    registration.publish(runner).await?;
    if profile::adopt_absent(paths, &guard, &writer).is_err() {
        return match registration.rollback(){Ok(())=>Err(OmmError::new(ErrorCode::Backend,false,"profile adoption failed; original registration file restored but job is unloaded; inspect registration before recovery")),Err(_)=>Err(OmmError::new(ErrorCode::Conflict,false,"profile adoption failed and registration changed; metadata retained; inspect registration before recovery"))};
    }
    Ok(())
}

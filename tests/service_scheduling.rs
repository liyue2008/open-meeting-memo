use async_trait::async_trait;
use omm::{
    core::{contracts::*, paths::*, runtime::*, store::Store},
    service::registration::*,
};
use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::Mutex,
};

#[derive(Default)]
struct Runner {
    file: PathBuf,
    calls: Mutex<Vec<(String, String)>>,
    fail_calls: Vec<usize>,
}
#[async_trait]
impl ServiceRunner for Runner {
    async fn run(&self, args: &[String]) -> Result<()> {
        let mut calls = self.calls.lock().unwrap();
        calls.push((args[0].clone(), fs::read_to_string(&self.file).unwrap()));
        if self.fail_calls.contains(&calls.len()) {
            Err(OmmError::new(
                ErrorCode::Backend,
                false,
                "fixture manager rejection",
            ))
        } else {
            Ok(())
        }
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    paths: Paths,
    location: RuntimeLocation,
    exe: PathBuf,
    file: PathBuf,
    backup: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("omm-reg-")
            .tempdir_in("/private/tmp")
            .unwrap();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        let paths = Paths::resolve(&home, &Default::default()).unwrap();
        paths.create_private_dirs().unwrap();
        let runtime = root.join("run");
        private_dir(&runtime).unwrap();
        let location = RuntimeLocation::resolve(
            &std::collections::BTreeMap::from([(
                "XDG_RUNTIME_DIR".into(),
                runtime.to_str().unwrap().into(),
            )]),
            || panic!("private runtime required"),
        )
        .unwrap();
        let exe = root.join("omm-fixture");
        write_private_new(&exe, b"not executed").unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
        let file = home.join("Library/LaunchAgents/org.openmeetingmemo.service.plist");
        let backup = file.with_extension("pre-standard.backup");
        Self {
            _temp: temp,
            root,
            home,
            paths,
            location,
            exe,
            file,
            backup,
        }
    }
    fn runner(&self, fail_calls: Vec<usize>) -> Runner {
        Runner {
            file: self.file.clone(),
            fail_calls,
            ..Default::default()
        }
    }
    async fn install(&self, r: &Runner) -> Result<()> {
        install_at(&self.home, &self.paths, &self.exe, Some(&self.location), r).await
    }
    async fn legacy(&self, r: &Runner) -> Vec<u8> {
        self.install(r).await.unwrap();
        let body = fs::read_to_string(&self.file).unwrap().replace(
            "<key>ProcessType</key><string>Standard</string>",
            "<key>ProcessType</key><string>Background</string>",
        );
        fs::write(&self.file, &body).unwrap();
        body.into_bytes()
    }
}
fn ops(r: &Runner) -> Vec<String> {
    r.calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| c.0.clone())
        .collect()
}

async fn install_with(f: &Fixture, r: &dyn ServiceRunner) -> Result<()> {
    install_at(&f.home, &f.paths, &f.exe, Some(&f.location), r).await
}
struct PausedRegistration {
    file: PathBuf,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    paused: std::sync::atomic::AtomicBool,
    job: Mutex<String>,
    calls: Mutex<Vec<String>>,
}
#[async_trait]
impl ServiceRunner for PausedRegistration {
    async fn run(&self, args: &[String]) -> Result<()> {
        self.calls.lock().unwrap().push(args[0].clone());
        if args[0] == "bootout" {
            self.job.lock().unwrap().clear();
        }
        if args[0] == "bootstrap" {
            if !self.paused.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.entered.notify_one();
                self.resume.notified().await;
                return Err(OmmError::new(
                    ErrorCode::Backend,
                    false,
                    "fixture activation rejected",
                ));
            }
            *self.job.lock().unwrap() = if fs::read_to_string(&self.file)
                .unwrap()
                .contains("<string>Standard</string>")
            {
                "Standard"
            } else {
                "Background"
            }
            .into();
        }
        Ok(())
    }
}
#[tokio::test]
async fn stable_registration_lock_covers_fresh_legacy_rollback_and_all_same_label_profiles() {
    for legacy in [false, true] {
        let f = Fixture::new();
        let setup = f.runner(vec![]);
        if legacy {
            f.legacy(&setup).await;
        }
        let r = PausedRegistration {
            file: f.file.clone(),
            entered: Default::default(),
            resume: Default::default(),
            paused: Default::default(),
            job: Mutex::new("Background".into()),
            calls: Default::default(),
        };
        let first = install_with(&f, &r);
        let second = async {
            r.entered.notified().await;
            assert_eq!(
                install_with(&f, &r).await.unwrap_err().code,
                ErrorCode::Conflict
            );
            assert_eq!(
                unregister(&f.home, &r).await.unwrap_err().code,
                ErrorCode::Conflict
            );
            assert_eq!(
                uninstall_at(&f.home, &f.paths, &f.location, &r)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            assert_eq!(
                start(&f.home, &r).await.unwrap_err().code,
                ErrorCode::Conflict
            );
            let mut other = f.paths.clone();
            other.state = f.root.join("other-state");
            private_dir(&f.root.join("other-run")).unwrap();
            let env = std::collections::BTreeMap::from([(
                "XDG_RUNTIME_DIR".into(),
                f.root.join("other-run").to_str().unwrap().into(),
            )]);
            let loc = RuntimeLocation::resolve(&env, || panic!()).unwrap();
            let e = install_at(&f.home, &other, &f.exe, Some(&loc), &r)
                .await
                .unwrap_err();
            assert!(e.message.contains("registration operation is active"));
            // The runtime lock is already free while bootstrap awaits, but registration remains locked.
            drop(RuntimeGuard::acquire(&f.location, &f.paths.state).unwrap());
            assert!(fs::read_to_string(&f.file)
                .unwrap()
                .contains("<string>Standard</string>"));
            r.resume.notify_one();
        };
        let (a, ()) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(first, second)
        })
        .await
        .unwrap();
        assert_eq!(a.unwrap_err().code, ErrorCode::Backend);
        let lock = f
            .file
            .parent()
            .unwrap()
            .join(".org.openmeetingmemo.service.registration.lock");
        let ino = fs::metadata(&lock).unwrap().ino();
        install_with(&f, &r).await.unwrap();
        assert_eq!(&*r.job.lock().unwrap(), "Standard");
        assert!(fs::read_to_string(&f.file)
            .unwrap()
            .contains("<string>Standard</string>"));
        assert_eq!(r.calls.lock().unwrap().last().unwrap(), "bootstrap");
        assert_eq!(fs::metadata(&lock).unwrap().ino(), ino);
        f.install(&setup).await.unwrap();
        unregister(&f.home, &setup).await.unwrap();
        assert_eq!(fs::metadata(&lock).unwrap().ino(), ino);
        f.install(&setup).await.unwrap();
        assert_eq!(fs::metadata(&lock).unwrap().ino(), ino);
    }
}
struct ReplacingRegistration {
    file: PathBuf,
    phase: &'static str,
    kind: u8,
    fail: bool,
    calls: Mutex<Vec<String>>,
}
#[async_trait]
impl ServiceRunner for ReplacingRegistration {
    async fn run(&self, args: &[String]) -> Result<()> {
        let bootstrap_count = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(args[0].clone());
            calls.iter().filter(|s| s.as_str() == "bootstrap").count()
        };
        if self.phase == "rollback" && args[0] == "bootstrap" && bootstrap_count == 1 {
            return Err(OmmError::new(
                ErrorCode::Backend,
                false,
                "fixture activation rejected",
            ));
        }
        if args[0] == self.phase
            || (self.phase == "rollback" && args[0] == "bootstrap" && bootstrap_count == 2)
        {
            tokio::task::yield_now().await;
            match self.kind {
                0 => {
                    let tmp = self.file.with_extension("foreign");
                    write_private_new(&tmp, b"foreign concurrent metadata").unwrap();
                    fs::rename(tmp, &self.file).unwrap();
                }
                1 => {
                    let tmp = self.file.with_extension("foreign");
                    write_private_new(&tmp, b"foreign concurrent metadata").unwrap();
                    fs::remove_file(&self.file).unwrap();
                    symlink(tmp, &self.file).unwrap();
                }
                2 => {
                    let parent = self.file.parent().unwrap();
                    fs::rename(parent, parent.with_extension("saved")).unwrap();
                    private_dir(parent).unwrap();
                    write_private_new(&self.file, b"foreign concurrent metadata").unwrap();
                }
                3 => fs::write(&self.file, b"foreign concurrent metadata").unwrap(),
                4 => {
                    let lib = self.file.parent().unwrap().parent().unwrap();
                    fs::rename(lib, lib.with_extension("saved")).unwrap();
                    private_dir(self.file.parent().unwrap()).unwrap();
                    write_private_new(&self.file, b"foreign concurrent metadata").unwrap();
                }
                _ => {
                    let parent = self.file.parent().unwrap();
                    fs::rename(parent, parent.with_extension("saved")).unwrap();
                    let foreign = parent.with_extension("foreign");
                    private_dir(&foreign).unwrap();
                    write_private_new(
                        &foreign.join(self.file.file_name().unwrap()),
                        b"foreign concurrent metadata",
                    )
                    .unwrap();
                    symlink(&foreign, parent).unwrap();
                }
            }
            if self.fail {
                return Err(OmmError::new(
                    ErrorCode::Backend,
                    false,
                    "fixture activation rejected",
                ));
            }
        }
        Ok(())
    }
}
#[tokio::test]
async fn anchored_publish_and_rollback_preserve_external_inode_content_symlink_and_parent_changes()
{
    for (phase, fail) in [
        ("bootout", false),
        ("bootstrap", false),
        ("bootstrap", true),
        ("rollback", false),
    ] {
        for kind in 0..6 {
            let f = Fixture::new();
            let init = f.runner(vec![]);
            let old = f.legacy(&init).await;
            let r = ReplacingRegistration {
                file: f.file.clone(),
                phase,
                kind,
                fail,
                calls: Default::default(),
            };
            let error = install_with(&f, &r).await.unwrap_err();
            if phase == "rollback" {
                assert!(error.message.contains("rollback metadata changed"));
            }
            assert_eq!(fs::read(&f.file).unwrap(), b"foreign concurrent metadata");
            assert_eq!(
                *r.calls.lock().unwrap(),
                if phase == "bootout" {
                    vec!["bootout"]
                } else if phase == "rollback" {
                    vec!["bootout", "bootstrap", "bootstrap"]
                } else {
                    vec!["bootout", "bootstrap"]
                }
            );
            let saved = if kind == 2 || kind == 5 {
                f.file
                    .parent()
                    .unwrap()
                    .with_extension("saved")
                    .join(f.backup.file_name().unwrap())
            } else if kind == 4 {
                f.home
                    .join("Library.saved/LaunchAgents")
                    .join(f.backup.file_name().unwrap())
            } else {
                f.backup.clone()
            };
            assert_eq!(fs::read(saved).unwrap(), old);
        }
    }
}
struct SameBytesReplacement {
    file: PathBuf,
    inode: std::sync::atomic::AtomicU64,
}
#[async_trait]
impl ServiceRunner for SameBytesReplacement {
    async fn run(&self, args: &[String]) -> Result<()> {
        if args[0] == "bootout" {
            let bytes = fs::read(&self.file).unwrap();
            let tmp = self.file.with_extension("same-bytes");
            write_private_new(&tmp, &bytes).unwrap();
            fs::rename(tmp, &self.file).unwrap();
            self.inode.store(
                fs::metadata(&self.file).unwrap().ino(),
                std::sync::atomic::Ordering::SeqCst,
            );
        }
        Ok(())
    }
}
#[tokio::test]
async fn inode_replacement_with_identical_bytes_is_not_overwritten() {
    let f = Fixture::new();
    let setup = f.runner(vec![]);
    let old = f.legacy(&setup).await;
    let before = fs::metadata(&f.file).unwrap().ino();
    let r = SameBytesReplacement {
        file: f.file.clone(),
        inode: Default::default(),
    };
    assert!(install_with(&f, &r).await.is_err());
    let after = r.inode.load(std::sync::atomic::Ordering::SeqCst);
    assert_ne!(before, after);
    assert_eq!(fs::metadata(&f.file).unwrap().ino(), after);
    assert_eq!(fs::read(&f.file).unwrap(), old);
    assert_eq!(fs::read(&f.backup).unwrap(), old);
}
#[tokio::test]
async fn fresh_failed_bootstrap_never_deletes_external_replacement() {
    let f = Fixture::new();
    let r = ReplacingRegistration {
        file: f.file.clone(),
        phase: "bootstrap",
        kind: 0,
        fail: true,
        calls: Default::default(),
    };
    assert!(install_with(&f, &r).await.is_err());
    assert_eq!(fs::read(&f.file).unwrap(), b"foreign concurrent metadata");
}
#[tokio::test]
async fn unregister_rechecks_expected_file_after_bootout_and_preserves_foreign_metadata() {
    let f = Fixture::new();
    let init = f.runner(vec![]);
    f.install(&init).await.unwrap();
    let r = ReplacingRegistration {
        file: f.file.clone(),
        phase: "bootout",
        kind: 0,
        fail: false,
        calls: Default::default(),
    };
    assert!(unregister(&f.home, &r).await.is_err());
    assert_eq!(fs::read(&f.file).unwrap(), b"foreign concurrent metadata");
}
#[tokio::test]
async fn registration_lock_unsafe_paths_are_rejected_without_unlink_or_follow() {
    for kind in 0..3 {
        let f = Fixture::new();
        let r = f.runner(vec![]);
        f.install(&r).await.unwrap();
        let lock = f
            .file
            .parent()
            .unwrap()
            .join(".org.openmeetingmemo.service.registration.lock");
        let target = f.root.join("not-a-lock");
        write_private_new(&target, b"unchanged target").unwrap();
        match kind {
            0 => {
                fs::remove_file(&lock).unwrap();
                symlink(&target, &lock).unwrap();
            }
            1 => fs::set_permissions(&lock, fs::Permissions::from_mode(0o666)).unwrap(),
            _ => {
                fs::remove_file(&lock).unwrap();
                fs::create_dir(&lock).unwrap();
            }
        }
        let ino = fs::symlink_metadata(&lock).unwrap().ino();
        assert!(f.install(&r).await.is_err());
        assert_eq!(ops(&r), ["bootstrap"]);
        assert_eq!(fs::symlink_metadata(&lock).unwrap().ino(), ino);
        assert_eq!(fs::read(target).unwrap(), b"unchanged target");
    }
}
#[tokio::test]
async fn readonly_durable_seal_gate_handles_fresh_legacy_old_schema_and_corruption() {
    for legacy in [false, true] {
        for case in 0..9 {
            let f = Fixture::new();
            let r = f.runner(vec![]);
            if legacy {
                f.legacy(&r).await;
            }
            let mut db = Store::open(&f.paths).unwrap();
            let id = uuid::Uuid::new_v4();
            let dir = f.paths.data.join("meetings").join(id.to_string());
            private_dir(&dir).unwrap();
            db.begin_segmented(
                &SealedMeeting {
                    session_id: id,
                    title: "Synthetic".into(),
                    directory: dir,
                    segments: vec![],
                    sealed_at_ms: 0,
                },
                &ConfigSnapshot::default(),
            )
            .unwrap();
            drop(db);
            let db_path = f.paths.state.join("state.sqlite");
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            match case {
                0 => (),
                1 => {
                    conn.execute("UPDATE segmented_sessions SET sealed=1", [])
                        .unwrap();
                }
                2 => {
                    conn.execute("DROP TABLE segmented_sessions", []).unwrap();
                }
                3 => {
                    conn.execute_batch("DROP TABLE segmented_sessions; CREATE TABLE segmented_sessions(other INTEGER)").unwrap();
                }
                4 => {
                    conn.execute_batch("DROP TABLE segmented_sessions; CREATE VIEW segmented_sessions AS SELECT 1 AS sealed").unwrap();
                }
                5 => {
                    conn.execute("UPDATE segmented_sessions SET sealed='invalid'", [])
                        .unwrap();
                }
                7 => {
                    conn.execute_batch("DROP TABLE segmented_sessions; CREATE TABLE segmented_sessions(sealed TEXT NOT NULL)").unwrap();
                }
                8 => {
                    conn.execute_batch("DROP TABLE segmented_sessions; CREATE TABLE segmented_sessions(sealed INTEGER)").unwrap();
                }
                _ => (),
            }
            drop(conn);
            if case == 6 {
                fs::write(&db_path, b"invalid sqlite fixture").unwrap();
            }
            let before = fs::read(&db_path).unwrap();
            let calls = ops(&r);
            let bytes = fs::read(&f.file).ok();
            let ino = fs::metadata(&f.file).ok().map(|m| m.ino());
            let result = f.install(&r).await;
            if case == 1 || case == 2 {
                result.unwrap();
            } else {
                assert!(result.is_err());
                assert_eq!(ops(&r), calls);
                assert_eq!(fs::read(&f.file).ok(), bytes);
                assert_eq!(fs::metadata(&f.file).ok().map(|m| m.ino()), ino);
                assert!(!f.backup.exists());
            }
            assert_eq!(fs::read(&db_path).unwrap(), before);
        }
    }
}
#[tokio::test]
async fn standard_no_reload_is_idempotent_even_with_durable_unsealed_session() {
    let f = Fixture::new();
    let r = f.runner(vec![]);
    f.install(&r).await.unwrap();
    let mut db = Store::open(&f.paths).unwrap();
    let id = uuid::Uuid::new_v4();
    let dir = f.paths.data.join("meetings").join(id.to_string());
    private_dir(&dir).unwrap();
    db.begin_segmented(
        &SealedMeeting {
            session_id: id,
            title: "Synthetic".into(),
            directory: dir,
            segments: vec![],
            sealed_at_ms: 0,
        },
        &ConfigSnapshot::default(),
    )
    .unwrap();
    drop(db);
    let before = fs::read(f.paths.state.join("state.sqlite")).unwrap();
    let ino = fs::metadata(&f.file).unwrap().ino();
    f.install(&r).await.unwrap();
    assert_eq!(ops(&r), ["bootstrap"]);
    assert_eq!(fs::metadata(&f.file).unwrap().ino(), ino);
    assert_eq!(
        fs::read(f.paths.state.join("state.sqlite")).unwrap(),
        before
    );
}

#[tokio::test]
async fn legacy_background_reload_changes_only_process_type_preserves_roots_and_is_idempotent() {
    let f = Fixture::new();
    let r = f.runner(vec![]);
    let legacy = f.legacy(&r).await;
    let config = f.paths.config.join("config.json");
    write_private_new(&config, b"unchanged fixture config").unwrap();
    let media = f.paths.data.join("legacy.wav");
    write_private_new(&media, b"unchanged synthetic bytes").unwrap();
    let db = Store::open(&f.paths).unwrap();
    drop(db);
    let before_db = fs::read(f.paths.state.join("state.sqlite")).unwrap();
    f.install(&r).await.unwrap();
    assert_eq!(ops(&r), ["bootstrap", "bootout", "bootstrap"]);
    let standard = fs::read_to_string(&f.file).unwrap();
    assert_eq!(
        standard,
        String::from_utf8(legacy.clone()).unwrap().replace(
            "<key>ProcessType</key><string>Background</string>",
            "<key>ProcessType</key><string>Standard</string>"
        )
    );
    assert_eq!(fs::read(&f.backup).unwrap(), legacy);
    assert_eq!(fs::metadata(&f.backup).unwrap().mode() & 0o777, 0o600);
    assert!(!f.backup.to_str().unwrap().ends_with(".plist"));
    assert_eq!(
        installed(&f.home).unwrap().unwrap().explicit_runtime_root,
        Some(f.root.join("run"))
    );
    assert_eq!(fs::read(&config).unwrap(), b"unchanged fixture config");
    assert_eq!(fs::read(&media).unwrap(), b"unchanged synthetic bytes");
    assert_eq!(
        fs::read(f.paths.state.join("state.sqlite")).unwrap(),
        before_db
    );
    let ino = fs::metadata(&f.file).unwrap().ino();
    f.install(&r).await.unwrap();
    assert_eq!(ops(&r).len(), 3);
    assert_eq!(fs::metadata(&f.file).unwrap().ino(), ino);
    assert_eq!(fs::read_to_string(&f.file).unwrap(), standard);
}

#[tokio::test]
async fn rejected_standard_bootstrap_restores_legacy_profile_and_allows_safe_retry() {
    let f = Fixture::new();
    let r = f.runner(vec![3]);
    let old = f.legacy(&r).await;
    assert_eq!(f.install(&r).await.unwrap_err().code, ErrorCode::Backend);
    assert_eq!(fs::read(&f.file).unwrap(), old);
    assert_eq!(fs::read(&f.backup).unwrap(), old);
    assert_eq!(ops(&r), ["bootstrap", "bootout", "bootstrap", "bootstrap"]);
    {
        let calls = r.calls.lock().unwrap();
        assert!(calls[2].1.contains("<string>Standard</string>"));
        assert!(calls[3].1.contains("<string>Background</string>"));
    }
    f.install(&r).await.unwrap();
    assert!(fs::read_to_string(&f.file)
        .unwrap()
        .contains("<string>Standard</string>"));
}

#[tokio::test]
async fn failed_rollback_keeps_owned_metadata_and_explicit_restart_diagnostic() {
    let f = Fixture::new();
    let r = f.runner(vec![3, 4]);
    let old = f.legacy(&r).await;
    let error = f.install(&r).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::Backend);
    assert!(error.message.contains("restored on disk"));
    assert_eq!(fs::read(&f.file).unwrap(), old);
    assert_eq!(fs::read(&f.backup).unwrap(), old);
}

#[tokio::test]
async fn bootout_rejection_never_replaces_the_old_registration() {
    let f = Fixture::new();
    let r = f.runner(vec![2]);
    let old = f.legacy(&r).await;
    assert_eq!(f.install(&r).await.unwrap_err().code, ErrorCode::Backend);
    assert_eq!(fs::read(&f.file).unwrap(), old);
    assert_eq!(fs::read(&f.backup).unwrap(), old);
    assert_eq!(ops(&r), ["bootstrap", "bootout"]);
}

#[tokio::test]
async fn foreign_custom_and_secret_environment_profiles_are_rejected_without_backup_or_runner() {
    let changes = [
        (
            "org.openmeetingmemo.service</string>",
            "foreign.service</string>",
        ),
        (
            "<string>--non-interactive</string>",
            "<string>--unsafe</string>",
        ),
        ("<string>Background</string>", "<string>Adaptive</string>"),
        (
            "<string>Background</string>",
            "<string>Interactive</string>",
        ),
        (
            "<key>RunAtLoad</key>",
            "<key>Nice</key><integer>-10</integer><key>RunAtLoad</key>",
        ),
        (
            "<key>HOME</key>",
            "<key>SECRET_TOKEN</key><string>fixture-secret-never-backed-up</string><key>HOME</key>",
        ),
        (
            "<key>Label</key>",
            "<key>Label</key><string>duplicate</string><key>Label</key>",
        ),
    ];
    for (from, to) in changes {
        let f = Fixture::new();
        let r = f.runner(vec![]);
        let old = f.legacy(&r).await;
        let custom = String::from_utf8(old).unwrap().replace(from, to);
        fs::write(&f.file, &custom).unwrap();
        let e = f.install(&r).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Conflict);
        assert!(!e.message.contains("fixture-secret"));
        assert_eq!(ops(&r), ["bootstrap"]);
        assert!(!f.backup.exists());
        assert_eq!(fs::read_to_string(&f.file).unwrap(), custom);
    }
}

#[tokio::test]
async fn symlink_parent_profile_executable_and_unsafe_executable_permissions_are_rejected() {
    for case in 0..7 {
        let f = Fixture::new();
        let r = f.runner(vec![]);
        let old = f.legacy(&r).await;
        match case {
            0 => {
                let real = f.root.join("original.plist");
                fs::rename(&f.file, &real).unwrap();
                symlink(real, &f.file).unwrap();
            }
            1 => {
                let dir = f.root.join("original-agents");
                fs::rename(f.file.parent().unwrap(), &dir).unwrap();
                symlink(dir, f.file.parent().unwrap()).unwrap();
            }
            2 => {
                let real = f.root.join("original-exe");
                fs::rename(&f.exe, &real).unwrap();
                symlink(real, &f.exe).unwrap();
            }
            3 => fs::set_permissions(&f.exe, fs::Permissions::from_mode(0o600)).unwrap(),
            4 => fs::set_permissions(&f.exe, fs::Permissions::from_mode(0o777)).unwrap(),
            5 => fs::set_permissions(&f.file, fs::Permissions::from_mode(0o666)).unwrap(),
            _ => fs::set_permissions(f.file.parent().unwrap(), fs::Permissions::from_mode(0o777))
                .unwrap(),
        }
        assert!(f.install(&r).await.is_err());
        assert_eq!(ops(&r), ["bootstrap"]);
        assert!(!f.backup.exists());
        assert_eq!(fs::read(&f.file).unwrap(), old);
    }
    let f = Fixture::new();
    let r = f.runner(vec![]);
    // Read-only metadata check of a root-owned executable; no chown/root operation.
    assert_ne!(fs::metadata("/usr/bin/true").unwrap().uid(), unsafe {
        libc::geteuid()
    });
    assert_eq!(
        install_at(
            &f.home,
            &f.paths,
            &PathBuf::from("/usr/bin/true"),
            Some(&f.location),
            &r
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::Permission
    );
    assert!(ops(&r).is_empty());
}

#[tokio::test]
async fn live_service_unsealed_recording_or_capture_endpoint_blocks_upgrade_before_bootout() {
    for case in 0..4 {
        let f = Fixture::new();
        let r = f.runner(vec![]);
        let old = f.legacy(&r).await;
        let guard = if case == 0 {
            Some(RuntimeGuard::acquire(&f.location, &f.paths.state).unwrap())
        } else {
            None
        };
        if case == 1 || case == 2 {
            let store = Store::open(&f.paths).unwrap();
            if case == 1 {
                store
                    .set_state("recording", &serde_json::json!({"synthetic":true}))
                    .unwrap();
            } else {
                store.set_state("capture_helper_expected", &true).unwrap();
            }
        }
        if case == 3 {
            private_dir(&f.location.path.join("capture")).unwrap();
            write_private_new(
                &f.location.path.join("capture/audio_daemon.sock"),
                b"stale or unknown endpoint",
            )
            .unwrap();
        }
        assert_eq!(f.install(&r).await.unwrap_err().code, ErrorCode::Conflict);
        drop(guard);
        assert_eq!(ops(&r), ["bootstrap"]);
        assert_eq!(fs::read(&f.file).unwrap(), old);
        assert!(!f.backup.exists());
    }
}

#[tokio::test]
async fn conflicting_backup_roots_or_executable_identity_never_get_overwritten() {
    let f = Fixture::new();
    let r = f.runner(vec![]);
    let old = f.legacy(&r).await;
    write_private_new(&f.backup, b"prior metadata preserved").unwrap();
    assert_eq!(f.install(&r).await.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(fs::read(&f.backup).unwrap(), b"prior metadata preserved");
    fs::remove_file(&f.backup).unwrap();
    let other_exe = f.root.join("another-exe");
    write_private_new(&other_exe, b"fixture").unwrap();
    fs::set_permissions(&other_exe, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        install_at(&f.home, &f.paths, &other_exe, Some(&f.location), &r)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let mut other_paths = f.paths.clone();
    other_paths.data = f.root.join("different-data");
    assert_eq!(
        install_at(&f.home, &other_paths, &f.exe, Some(&f.location), &r)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(ops(&r), ["bootstrap"]);
    assert_eq!(fs::read(&f.file).unwrap(), old);
    assert!(!f.backup.exists());
}

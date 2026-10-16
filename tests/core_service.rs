use async_trait::async_trait;
use omm::{
    core::{contracts::*, paths::*},
    service::registration::*,
};
use std::{fs, os::unix::fs::PermissionsExt, sync::Mutex};
#[derive(Default)]
struct Runner {
    calls: Mutex<Vec<Vec<String>>>,
    fail: bool,
}
#[async_trait]
impl ServiceRunner for Runner {
    async fn run(&self, args: &[String]) -> Result<()> {
        self.calls.lock().unwrap().push(args.to_vec());
        if self.fail {
            Err(OmmError::new(
                ErrorCode::Backend,
                false,
                "fixture registration failure",
            ))
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn launchagent_registration_uses_fixed_argv_temp_home_and_never_deletes_data() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let home = root.join("home");
    let p = Paths::resolve(&home, &Default::default()).unwrap();
    p.create_private_dirs().unwrap();
    let exe = root.join("omm&fixture");
    fs::write(&exe, b"synthetic executable").unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
    let r = Runner::default();
    install(&home, &p, &exe, &r).await.unwrap();
    assert_eq!(installed(&home).unwrap().unwrap().paths, p);
    let file = home.join("Library/LaunchAgents/org.openmeetingmemo.service.plist");
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("<key>ProcessType</key><string>Standard</string>"));
    assert!(text.contains("omm&amp;fixture"));
    assert!(text.contains("<string>service</string><string>run</string>"));
    assert!(!text.contains("XDG_RUNTIME_DIR"));
    assert_eq!(r.calls.lock().unwrap()[0][0], "bootstrap");
    let before = fs::read(&file).unwrap();
    install(&home, &p, &exe, &r).await.unwrap();
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(r.calls.lock().unwrap().len(), 1);
    start(&home, &r).await.unwrap();
    assert_eq!(r.calls.lock().unwrap()[1][0], "kickstart");
    let data = p.data.join("original-audio.wav");
    fs::write(&data, b"synthetic preserved data").unwrap();
    unregister(&home, &r).await.unwrap();
    assert_eq!(r.calls.lock().unwrap()[2][0], "bootout");
    assert!(installed(&home).unwrap().is_none());
    assert!(data.exists());
    assert!(p.state.exists());
    assert!(p.config.exists());
}
#[tokio::test]
async fn rejected_registration_does_not_leave_false_installed_profile() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let home = root.join("home");
    let p = Paths::resolve(&home, &Default::default()).unwrap();
    p.create_private_dirs().unwrap();
    let exe = root.join("omm-fixture");
    fs::write(&exe, b"synthetic").unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
    let r = Runner {
        fail: true,
        ..Default::default()
    };
    assert_eq!(
        install(&home, &p, &exe, &r).await.unwrap_err().code,
        ErrorCode::Backend
    );
    assert!(installed(&home).unwrap().is_none());
}

#[tokio::test]
async fn installed_explicit_runtime_is_frozen_validated_and_missing_root_can_degrade() {
    use omm::core::runtime::RuntimeLocation;
    use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};
    let t = tempfile::Builder::new()
        .prefix("omm-s-")
        .tempdir_in("/private/tmp")
        .unwrap();
    fs::set_permissions(t.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let root = t.path().canonicalize().unwrap();
    let home = root.join("home");
    let paths = Paths::resolve(&home, &BTreeMap::new()).unwrap();
    paths.create_private_dirs().unwrap();
    let explicit = root.join("runtime&fixture");
    private_dir(&explicit).unwrap();
    let env = BTreeMap::from([("XDG_RUNTIME_DIR".into(), explicit.to_str().unwrap().into())]);
    let selected = RuntimeLocation::resolve(&env, || panic!("must use explicit root")).unwrap();
    let executable = root.join("omm-fixture-bin");
    fs::write(&executable, b"fixture").unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let runner = Runner::default();
    install_at(&home, &paths, &executable, Some(&selected), &runner)
        .await
        .unwrap();
    let profile = installed(&home).unwrap().unwrap();
    assert_eq!(profile.explicit_runtime_root, Some(explicit.clone()));
    profile.check_runtime(&selected).unwrap();
    let plist =
        fs::read_to_string(home.join("Library/LaunchAgents/org.openmeetingmemo.service.plist"))
            .unwrap();
    assert!(plist.contains("<key>XDG_RUNTIME_DIR</key>"));
    assert!(plist.contains("runtime&amp;fixture"));
    let replacement = RuntimeLocation::resolve(&BTreeMap::new(), || Ok(root.clone())).unwrap();
    assert_eq!(
        profile.check_runtime(&replacement).unwrap_err().code,
        ErrorCode::Conflict
    );
    fs::set_permissions(&explicit, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        profile.check_runtime(&replacement).unwrap_err().code,
        ErrorCode::Permission
    );
    fs::remove_dir(&explicit).unwrap();
    profile.check_runtime(&replacement).unwrap();
    unregister(&home, &runner).await.unwrap();
    assert!(paths.data.is_dir());
    assert!(paths.state.is_dir());
}

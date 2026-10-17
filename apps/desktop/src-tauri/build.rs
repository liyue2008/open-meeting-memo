include!("../../../contracts/desktop/v1/commands.generated.rs");
fn main() {
    println!("cargo:rerun-if-env-changed=OMM_SOURCE_REVISION");
    // Generate explicit permissions for custom commands, without granting any of them.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMAND_NAMES)),
    )
    .expect("desktop ACL/config build");
}

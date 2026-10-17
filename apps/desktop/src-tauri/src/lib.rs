//! Desktop entry point. Application commands are supplied by the native owner.
//! Foundation deliberately registers no business handler or fallback success.
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("desktop application runtime failed");
}

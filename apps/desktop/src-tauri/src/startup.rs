//! Explicit per-launch WK isolation; never selects or inspects the default store.
use std::ffi::OsString;
use tauri::Manager;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Options {
    ephemeral: bool,
    diagnostics: bool,
}
impl Options {
    pub fn from_process() -> Result<Self, &'static str> {
        Self::parse(
            std::env::args_os().skip(1),
            std::env::var_os("OMM_WKWEBVIEW_STORE_MANIFEST").is_some(),
            cfg!(feature = "native-bridge-fixture"),
        )
    }
    fn parse(
        args: impl IntoIterator<Item = OsString>,
        owned_store: bool,
        fixture_build: bool,
    ) -> Result<Self, &'static str> {
        let mut options = Self::default();
        for arg in args {
            match arg.to_str() {
                Some("--ephemeral-webview") if !options.ephemeral => options.ephemeral = true,
                Some("--webview-store-diagnostics") if !options.diagnostics => {
                    options.diagnostics = true
                }
                _ => return Err("unknown or repeated desktop startup argument"),
            }
        }
        if options.diagnostics && !options.ephemeral {
            return Err("webview store diagnostics require --ephemeral-webview");
        }
        if owned_store && (options.ephemeral || !fixture_build) {
            return Err("owned fixture store conflicts with production webview startup");
        }
        if options.ephemeral && !cfg!(target_os = "macos") {
            return Err("ephemeral webview startup requires macOS");
        }
        Ok(options)
    }
    pub fn configure(&self, context: &mut tauri::Context<tauri::Wry>) -> Result<(), &'static str> {
        if !self.ephemeral {
            return Ok(());
        }
        let windows = &mut context.config_mut().app.windows;
        if windows.len() != 1
            || windows[0].label != "main"
            || !windows[0].create
            || windows[0].data_store_identifier.is_some()
        {
            return Err("ephemeral startup requires one automatically created main webview without a store identifier");
        }
        windows[0].incognito = true;
        Ok(())
    }
    pub fn check_support(&self) -> Result<(), &'static str> {
        if !self.ephemeral {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            use objc2::{msg_send, runtime::AnyClass};
            let class = AnyClass::get(c"WKWebsiteDataStore")
                .ok_or("WKWebsiteDataStore unavailable; ephemeral startup refused")?;
            // This asks about an API, not any user's/default data store.
            let supported: bool = unsafe {
                msg_send![class, respondsToSelector: objc2::sel!(nonPersistentDataStore)]
            };
            if supported {
                return Ok(());
            }
        }
        Err("nonpersistent WK store unavailable; startup refused")
    }
    pub fn ready(self, app: &tauri::AppHandle) {
        if !self.ephemeral {
            return;
        }
        let Some(window) = app.get_webview_window("main") else {
            eprintln!("ephemeral main webview missing; startup refused");
            app.exit(3);
            return;
        };
        let handle = app.clone();
        if window
            .with_webview(move |view| {
                #[cfg(target_os = "macos")]
                let verified = unsafe {
                    use objc2::{msg_send, runtime::AnyObject};
                    let native = view.inner().cast::<AnyObject>();
                    let configuration: *mut AnyObject = msg_send![native, configuration];
                    let store: *mut AnyObject = msg_send![configuration, websiteDataStore];
                    if store.is_null() {
                        false
                    } else {
                        let persistent: bool = msg_send![store, isPersistent];
                        !persistent
                    }
                };
                #[cfg(not(target_os = "macos"))]
                let verified = false;
                if !verified {
                    eprintln!("actual WK store is not nonpersistent; startup refused");
                    handle.exit(3);
                } else if self.diagnostics {
                    // Exactly one Ready event, no content, paths, eval, IPC or output-file option.
                    println!(
                        "{}",
                        serde_json::json!({"schemaVersion":1,"kind":"webview-store","mode":"ephemeral","label":"main","persistent":false,"pid":std::process::id()})
                    );
                }
            })
            .is_err()
        {
            eprintln!("cannot verify ephemeral WK store; startup refused");
            app.exit(3);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str], owned: bool, fixture: bool) -> Result<Options, &'static str> {
        Options::parse(args.iter().map(OsString::from), owned, fixture)
    }
    #[test]
    fn production_ephemeral_startup_is_wired_outside_fixture_configuration() {
        let source = include_str!("lib.rs");
        let run = &source[source.find("pub fn run()").unwrap()..];
        assert!(
            run.starts_with("pub fn run() {\n    let startup = startup::Options::from_process()")
        );
        let configure = run.find("startup.configure(&mut context)").unwrap();
        assert!(configure < run.find(".build(context)").unwrap());
        assert!(run.contains("startup.ready(handle)"));
    }
    #[test]
    fn production_ephemeral_startup_preserves_default_and_sets_opt_in_before_runtime() {
        let mut context = crate::context();
        let before = serde_json::to_value(context.config()).unwrap();
        parse(&[], false, false)
            .unwrap()
            .configure(&mut context)
            .unwrap();
        assert_eq!(serde_json::to_value(context.config()).unwrap(), before);
        assert!(!context.config().app.windows[0].incognito);
        parse(&["--ephemeral-webview"], false, false)
            .unwrap()
            .configure(&mut context)
            .unwrap();
        assert!(context.config().app.windows[0].incognito);
        assert!(context.config().app.windows[0]
            .data_store_identifier
            .is_none());
        let mut expected = before;
        expected["app"]["windows"][0]["incognito"] = true.into();
        assert_eq!(serde_json::to_value(context.config()).unwrap(), expected);
    }
    #[test]
    fn production_ephemeral_startup_rejects_ambiguous_and_unsupported_requests() {
        for args in [
            vec!["--webview-store-diagnostics"],
            vec!["--ephemeral-webview", "--ephemeral-webview"],
            vec![
                "--ephemeral-webview",
                "--webview-store-diagnostics",
                "--webview-store-diagnostics",
            ],
            vec!["--ephemeral-webview=1"],
            vec!["--ephemeral-webveiw"],
            vec!["--unknown"],
            vec!["-psn_invalid"],
        ] {
            assert!(parse(&args, false, false).is_err(), "{args:?}");
        }
        assert!(parse(&["--ephemeral-webview"], true, true).is_err());
        assert!(parse(&[], true, false).is_err());
        assert!(
            parse(&[], true, true).is_ok(),
            "owned UUID fixture default must survive"
        );
        assert!(
            parse(
                &["--webview-store-diagnostics", "--ephemeral-webview"],
                false,
                false
            )
            .unwrap()
            .diagnostics
        );
        assert!(Options::parse(
            [OsString::from("--ephemeral-webview"), {
                use std::os::unix::ffi::OsStringExt;
                OsString::from_vec(vec![255])
            }],
            false,
            false
        )
        .is_err());
    }
    #[test]
    fn production_ephemeral_startup_refuses_conflicting_window_configurations() {
        for defect in ["extra", "label", "manual", "identifier"] {
            let mut context = crate::context();
            let windows = &mut context.config_mut().app.windows;
            match defect {
                "extra" => windows.push(windows[0].clone()),
                "label" => windows[0].label = "other".into(),
                "manual" => windows[0].create = false,
                "identifier" => windows[0].data_store_identifier = Some([1; 16]),
                _ => unreachable!(),
            }
            let before = serde_json::to_value(context.config()).unwrap();
            assert!(parse(&["--ephemeral-webview"], false, false)
                .unwrap()
                .configure(&mut context)
                .is_err());
            assert_eq!(serde_json::to_value(context.config()).unwrap(), before);
        }
    }
}

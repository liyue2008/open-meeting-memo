//! Source-named checks for the production capability, without broad core defaults.
#[cfg(test)]
mod tests {
    #[test]
    fn main_close_permissions_are_local_and_exact() {
        use tauri::ipc::Origin;
        let mut context = crate::context();
        let authority = context.runtime_authority_mut();
        let remote = Origin::Remote {
            url: "https://example.invalid/".parse().unwrap(),
        };
        for command in [
            "plugin:event|listen",
            "plugin:event|unlisten",
            "plugin:window|destroy",
        ] {
            assert!(
                authority
                    .resolve_access(command, "main", "main", &Origin::Local)
                    .is_some(),
                "{command}"
            );
            assert!(
                authority
                    .resolve_access(command, "unknown", "unknown", &Origin::Local)
                    .is_none(),
                "{command}"
            );
            assert!(
                authority
                    .resolve_access(command, "main", "main", &remote)
                    .is_none(),
                "{command}"
            );
        }
        for command in [
            "plugin:event|emit",
            "plugin:event|emit_to",
            "plugin:window|close",
            "plugin:window|create",
            "plugin:window|set_title",
        ] {
            assert!(
                authority
                    .resolve_access(command, "main", "main", &Origin::Local)
                    .is_none(),
                "{command}"
            );
        }
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
        let permissions = capability["permissions"].as_array().unwrap();
        let business: Vec<_> = permissions
            .iter()
            .filter_map(|p| p.as_str())
            .filter(|p| !p.starts_with("core:"))
            .collect();
        assert_eq!(business.len(), 36);
        assert_eq!(permissions.len(), 39);
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
        assert_eq!(capability["local"], true);
        assert!(capability.get("remote").is_none());
    }
}

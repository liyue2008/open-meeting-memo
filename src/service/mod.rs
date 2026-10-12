use crate::core::{contracts::*, paths::Paths};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceProfile {
    pub schema_version: u32,
    pub paths: Paths,
    #[serde(default)]
    pub explicit_runtime_root: Option<std::path::PathBuf>,
}
impl ServiceProfile {
    pub fn new(paths: Paths) -> Self {
        Self {
            schema_version: 1,
            paths,
            explicit_runtime_root: None,
        }
    }
    pub fn new_at(paths: Paths, location: &crate::core::runtime::RuntimeLocation) -> Self {
        let mut profile = Self::new(paths);
        if location.source == "explicit_xdg" {
            profile.explicit_runtime_root = location.path.parent().map(ToOwned::to_owned);
        }
        profile
    }
    pub fn check_runtime(&self, location: &crate::core::runtime::RuntimeLocation) -> Result<()> {
        if let Some(root) = &self.explicit_runtime_root {
            let env = std::collections::BTreeMap::from([(
                "XDG_RUNTIME_DIR".into(),
                root.to_str()
                    .ok_or_else(|| OmmError::invalid("profile runtime must be UTF-8"))?
                    .into(),
            )]);
            let expected = crate::core::runtime::RuntimeLocation::resolve(&env, || {
                if location.source == "macos_replacement" {
                    location
                        .path
                        .parent()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| OmmError::invalid("replacement runtime parent missing"))
                } else {
                    Err(OmmError::new(
                        ErrorCode::Conflict,
                        false,
                        "installed explicit runtime is missing; use macOS replacement",
                    ))
                }
            })?;
            if expected.path != location.path {
                return Err(OmmError::new(ErrorCode::Conflict, false, "shell runtime conflicts with installed service profile; use the installed explicit XDG runtime"));
            }
        }
        Ok(())
    }
    pub fn check_roots(&self, paths: &Paths) -> Result<()> {
        if self.schema_version != 1 {
            return Err(OmmError::invalid("unsupported service profile"));
        }
        if self.paths.config != paths.config
            || self.paths.data != paths.data
            || self.paths.state != paths.state
            || self.paths.cache != paths.cache
        {
            return Err(OmmError::new(
                ErrorCode::Conflict,
                false,
                "shell XDG roots conflict with installed service profile",
            ));
        }
        Ok(())
    }
}

pub mod daemon;
pub mod registration;

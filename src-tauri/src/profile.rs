//! Compile-time application profile. Production locations are compatibility contracts.
use std::path::{Path, PathBuf};

#[cfg(all(feature = "dev-app", feature = "e2e-harness"))]
compile_error!("dev-app cannot include the e2e-harness");

pub(crate) const IS_DEV: bool = cfg!(feature = "dev-app");
pub(crate) const APP_NAME: &str = if IS_DEV { "Humhum Dev" } else { "Humhum" };
pub(crate) const IDENTIFIER: &str = if IS_DEV {
    "com.kacheo.humhum.dev"
} else {
    "com.kacheo.humhum"
};
#[cfg(not(test))]
pub(crate) const KEYCHAIN_SERVICE: &str = if IS_DEV {
    "humhum-api-dev"
} else {
    "humhum-api"
};
#[cfg(test)]
pub(crate) const KEYCHAIN_SERVICE: &str = "humhum-api-test";

fn storage_root(home: &Path, dev: bool) -> PathBuf {
    home.join("Library/Application Support")
        .join(if dev { "humhum-dev" } else { "humhum" })
}

pub(crate) fn data_root(home: &Path) -> PathBuf {
    storage_root(home, IS_DEV)
}

pub(crate) fn validate_identifier(identifier: &str) -> Result<(), &'static str> {
    if identifier == IDENTIFIER {
        Ok(())
    } else {
        Err("Humhum profile/config mismatch: use the Dev overlay with dev-app, or the production config without it")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn profiles_write_to_separate_directories() {
        assert_eq!(
            storage_root(Path::new("/Users/test"), false),
            Path::new("/Users/test/Library/Application Support/humhum")
        );
        assert_eq!(
            storage_root(Path::new("/Users/test"), true),
            Path::new("/Users/test/Library/Application Support/humhum-dev")
        );
    }

    #[test]
    fn mismatched_bundle_identity_is_rejected() {
        let expected = if cfg!(feature = "dev-app") {
            "com.kacheo.humhum.dev"
        } else {
            "com.kacheo.humhum"
        };
        assert!(validate_identifier(expected).is_ok());
        for id in [
            "com.example.other",
            "",
            if cfg!(feature = "dev-app") {
                "com.kacheo.humhum"
            } else {
                "com.kacheo.humhum.dev"
            },
        ] {
            assert!(validate_identifier(id).is_err());
        }
    }
}

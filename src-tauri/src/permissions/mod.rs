use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
pub mod macos;

/// macOS permission grants required for dictation, as reported by the
/// [`Permissions::check`] trait. Each field maps to one System Settings pane the
/// onboarding `PermissionsStep` links to: microphone (AVCaptureDevice),
/// accessibility (AXIsProcessTrusted), and input monitoring (IOHIDCheckAccess).
/// Serializes with camelCase field names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsState {
    pub microphone: bool,
    pub accessibility: bool,
    pub input_monitoring: bool,
}

impl PermissionsState {
    /// True only when every required permission is granted.
    pub fn all_granted(&self) -> bool {
        self.microphone && self.accessibility && self.input_monitoring
    }
}

/// Platform permission checking. macOS impl in [`macos`]; see LLD §3.3.
///
/// `request_microphone` triggers the system mic prompt (only microphone can be
/// requested programmatically). Accessibility and input monitoring cannot be
/// requested — the `open_*` methods deep-link the user to the relevant System
/// Settings pane instead.
pub trait Permissions: Send + Sync {
    fn check(&self) -> PermissionsState;
    /// Returns `Err(())` if the request could not be initiated. The unit error
    /// matches the LLD §3.3 trait signature — the grant outcome is reported
    /// asynchronously via [`Permissions::check`], not this return value.
    #[allow(clippy::result_unit_err)]
    fn request_microphone(&self) -> Result<(), ()>;
    fn open_accessibility_settings(&self);
    fn open_input_monitoring_settings(&self);
}

/// Construct the platform `Permissions` implementation (LLD §10 factory). In
/// debug builds, `HUMHUM_E2E_FORCE_PERMISSIONS=1` swaps in
/// [`ForcedPermissions`] (all-granted) so the onboarding wizard can be walked
/// to Finish in CI, where the three real macOS TCC grants can't be obtained
/// non-interactively.
#[cfg(target_os = "macos")]
pub fn create() -> Box<dyn Permissions> {
    #[cfg(debug_assertions)]
    if std::env::var("HUMHUM_E2E_FORCE_PERMISSIONS").as_deref() == Ok("1") {
        return Box::new(ForcedPermissions);
    }
    Box::new(macos::MacosPermissions)
}

/// E2E-only: reports every permission as granted and no-ops the two settings
/// deep-links, so the onboarding gate is testable without real TCC grants.
#[cfg(debug_assertions)]
struct ForcedPermissions;

#[cfg(debug_assertions)]
impl Permissions for ForcedPermissions {
    fn check(&self) -> PermissionsState {
        PermissionsState {
            microphone: true,
            accessibility: true,
            input_monitoring: true,
        }
    }
    fn request_microphone(&self) -> Result<(), ()> {
        Ok(())
    }
    fn open_accessibility_settings(&self) {}
    fn open_input_monitoring_settings(&self) {}
}

/// Non-macOS fallback: reports nothing granted so callers compile and behave
/// safely off-platform (the app ships macOS-only).
#[cfg(not(target_os = "macos"))]
pub fn create() -> Box<dyn Permissions> {
    Box::new(StubPermissions)
}

#[cfg(not(target_os = "macos"))]
struct StubPermissions;

#[cfg(not(target_os = "macos"))]
impl Permissions for StubPermissions {
    fn check(&self) -> PermissionsState {
        PermissionsState {
            microphone: false,
            accessibility: false,
            input_monitoring: false,
        }
    }
    fn request_microphone(&self) -> Result<(), ()> {
        Err(())
    }
    fn open_accessibility_settings(&self) {}
    fn open_input_monitoring_settings(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_state_round_trip_and_wire_keys() {
        let state = PermissionsState {
            microphone: true,
            accessibility: false,
            input_monitoring: true,
        };
        let value = serde_json::to_value(state).expect("serialize");
        // input_monitoring renames to camelCase inputMonitoring on the wire.
        assert_eq!(value["microphone"], true);
        assert_eq!(value["accessibility"], false);
        assert_eq!(value["inputMonitoring"], true);
        assert!(value.get("input_monitoring").is_none());

        let back: PermissionsState = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back, state);
    }

    /// Exercises the E2E-only stub directly (same-module access to the
    /// private type) rather than through `create()`'s env-var switch, so this
    /// carries no ambient-environment mutation risk.
    #[cfg(debug_assertions)]
    #[test]
    fn forced_permissions_reports_all_granted_and_noops_settings_links() {
        let perms = ForcedPermissions;
        assert_eq!(
            perms.check(),
            PermissionsState {
                microphone: true,
                accessibility: true,
                input_monitoring: true,
            }
        );
        assert!(perms.request_microphone().is_ok());
        perms.open_accessibility_settings(); // no-op; must not panic
        perms.open_input_monitoring_settings(); // no-op; must not panic
    }
}

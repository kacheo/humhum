pub use error::HotkeyError;

use serde::{Deserialize, Serialize};

mod error {
    #[derive(Debug, PartialEq)]
    pub enum HotkeyError {
        PermissionDenied,
        ConflictWith(String),
        InvalidCombo(String),
        TapCreationFailed,
    }

    impl HotkeyError {
        /// Stable snake_case wire code (LLD §601). `conflict_with`/`invalid_combo`
        /// carry the offending name/reason as a `:`-suffix.
        pub fn code(&self) -> String {
            match self {
                HotkeyError::PermissionDenied => "permission_denied".to_string(),
                HotkeyError::ConflictWith(s) => format!("conflict_with:{s}"),
                HotkeyError::InvalidCombo(s) => format!("invalid_combo:{s}"),
                HotkeyError::TapCreationFailed => "tap_creation_failed".to_string(),
            }
        }
    }

    impl std::fmt::Display for HotkeyError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                HotkeyError::PermissionDenied => write!(f, "permission denied"),
                HotkeyError::ConflictWith(s) => write!(f, "conflicts with: {s}"),
                HotkeyError::InvalidCombo(s) => write!(f, "invalid combo: {s}"),
                HotkeyError::TapCreationFailed => write!(f, "tap creation failed"),
            }
        }
    }
}

/// A keyboard modifier. Serializes snake_case: `cmd`, `option`, `shift`, `ctrl`, `fn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Cmd,
    Option,
    Shift,
    Ctrl,
    Fn,
}

impl Modifier {
    /// Parse a modifier token (canonical names + common aliases).
    fn parse(token: &str) -> Option<Modifier> {
        match token {
            "Cmd" | "Command" | "Super" => Some(Modifier::Cmd),
            "Option" | "Alt" => Some(Modifier::Option),
            "Shift" => Some(Modifier::Shift),
            "Ctrl" | "Control" => Some(Modifier::Ctrl),
            "Fn" => Some(Modifier::Fn),
            _ => None,
        }
    }
}

/// A parsed hotkey combination, e.g. `key: "Space"`, `modifiers: [Option, Shift]`.
/// A **modifier-only** combo (push-to-talk on a lone modifier, e.g. hold Option)
/// has an empty `key` and a non-empty `modifiers`.
/// Serializes with camelCase field names to match the TS contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCombo {
    pub key: String,
    pub modifiers: Vec<Modifier>,
}

impl HotkeyCombo {
    /// True when the combo has no non-modifier key — the modifiers alone act as
    /// the push-to-talk trigger (press to start, release to stop).
    pub fn is_modifier_only(&self) -> bool {
        self.key.is_empty()
    }
}

/// Named keys supported in a combo (beyond single letters/digits).
const NAMED_KEYS: &[&str] = &[
    "Space", "Return", "Enter", "Tab", "Escape", "Delete", "Up", "Down", "Left", "Right", "F1",
    "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
];

fn is_valid_key(key: &str) -> bool {
    (key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric())) || NAMED_KEYS.contains(&key)
}

/// Collect, de-dup, and validate a run of modifier tokens (order preserved).
fn parse_modifiers(tokens: &[&str]) -> Result<Vec<Modifier>, HotkeyError> {
    let mut modifiers: Vec<Modifier> = Vec::with_capacity(tokens.len());
    for m in tokens {
        let parsed = Modifier::parse(m)
            .ok_or_else(|| HotkeyError::InvalidCombo(format!("unknown modifier '{m}'")))?;
        if !modifiers.contains(&parsed) {
            modifiers.push(parsed); // dedup repeated modifiers, preserve order
        }
    }
    Ok(modifiers)
}

/// Parse a `+`-joined combo into a [`HotkeyCombo`].
///
/// Two shapes are accepted:
/// - **Keyed** (`"Option+Shift+Space"`): the last token is the key, the rest are
///   modifiers. Requires at least one modifier and a recognized key — a bare key
///   (`"Space"`) is too easy to trigger accidentally and is rejected.
/// - **Modifier-only** (`"Option"`, `"Option+Shift"`): every token is a modifier;
///   the modifiers alone are the push-to-talk trigger and `key` is empty. Since no
///   token is both a key and a modifier, the two shapes never collide.
pub fn parse_combo(combo: &str) -> Result<HotkeyCombo, HotkeyError> {
    let tokens: Vec<&str> = combo
        .split('+')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return Err(HotkeyError::InvalidCombo("empty combo".into()));
    }

    // Modifier-only: every token parses as a modifier (e.g. "Option", "Option+Shift").
    if tokens.iter().all(|t| Modifier::parse(t).is_some()) {
        return Ok(HotkeyCombo {
            key: String::new(),
            modifiers: parse_modifiers(&tokens)?,
        });
    }

    // Keyed: last token is the key, the rest are modifiers.
    let (key, mods) = tokens.split_last().expect("tokens is non-empty");
    if mods.is_empty() {
        return Err(HotkeyError::InvalidCombo(format!(
            "'{combo}' needs at least one modifier"
        )));
    }
    if !is_valid_key(key) {
        return Err(HotkeyError::InvalidCombo(format!("unknown key '{key}'")));
    }

    Ok(HotkeyCombo {
        key: (*key).to_string(),
        modifiers: parse_modifiers(mods)?,
    })
}

/// System shortcuts a user combo must not collide with. Each entry is
/// `(modifiers, key, name)`; modifier order is not significant.
const KNOWN_CONFLICTS: &[(&[Modifier], &str, &str)] = &[
    (&[Modifier::Cmd], "Space", "Spotlight"),
    (&[Modifier::Cmd], "Tab", "App Switcher"),
    (&[Modifier::Ctrl], "Up", "Mission Control"),
    (&[Modifier::Ctrl], "Down", "Application Windows"),
    (&[Modifier::Ctrl], "Left", "Move Left a Space"),
    (&[Modifier::Ctrl], "Right", "Move Right a Space"),
];

fn same_modifier_set(a: &[Modifier], b: &[Modifier]) -> bool {
    a.len() == b.len() && a.iter().all(|m| b.contains(m))
}

/// Reject a combo that collides with a well-known system shortcut.
pub fn check_conflicts(combo: &HotkeyCombo) -> Result<(), HotkeyError> {
    for (mods, key, name) in KNOWN_CONFLICTS {
        if combo.key == *key && same_modifier_set(&combo.modifiers, mods) {
            return Err(HotkeyError::ConflictWith((*name).to_string()));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Start,
    Stop,
    Cancel,
}

/// What the CGEventTap callback observed, reduced to combo-relevant cases so the
/// decision logic ([`classify`]) stays platform-independent and unit-testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEvent {
    /// The target key went down while all target modifiers are held.
    ComboPressed,
    /// The target key (or a required modifier) was released.
    ComboReleased,
    /// Escape went down.
    EscapePressed,
    /// Any other key event.
    Other,
}

/// The callback's decision: which event to emit (if any), the new `is_holding`
/// state, and whether to swallow the OS event so it doesn't reach the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapDecision {
    pub event: Option<HotkeyEvent>,
    pub holding: bool,
    pub swallow: bool,
}

/// Map a raw keyboard event against the target combo into a [`KeyEvent`].
/// Pure (no FFI) so the masking logic is unit-testable; `macos.rs` supplies the
/// CoreGraphics keycodes/flag masks.
///
/// `target_keycode` is `None` for a **modifier-only** combo (the modifiers alone
/// are the trigger); `Some(kc)` for a keyed combo.
///
/// Keyed combo:
/// - target key down with **all** target modifiers held → `ComboPressed`
/// - target key up, or a required modifier released (flagsChanged) → `ComboReleased`
///
/// Modifier-only combo (driven entirely by flagsChanged):
/// - flagsChanged with **all** target modifiers held → `ComboPressed`
///   (held-key repeat is suppressed downstream by [`classify`])
/// - flagsChanged with a target modifier released → `ComboReleased`
///
/// Either shape: Escape down → `EscapePressed`.
#[allow(clippy::too_many_arguments)]
pub fn observe(
    is_key_down: bool,
    is_flags_changed: bool,
    keycode: u16,
    active_mods_mask: u64,
    target_keycode: Option<u16>,
    target_mods_mask: u64,
    escape_keycode: u16,
) -> KeyEvent {
    let all_mods_held = active_mods_mask & target_mods_mask == target_mods_mask;
    if is_key_down && keycode == escape_keycode {
        return KeyEvent::EscapePressed;
    }
    match target_keycode {
        Some(target) => {
            // Released = the target key went up, or a required modifier was let go.
            let key_released = !is_key_down && !is_flags_changed && keycode == target;
            let modifier_released = is_flags_changed && !all_mods_held;
            if is_key_down && keycode == target && all_mods_held {
                KeyEvent::ComboPressed
            } else if key_released || modifier_released {
                KeyEvent::ComboReleased
            } else {
                KeyEvent::Other
            }
        }
        None => {
            // Modifier-only: the combo lives entirely in the modifier flags.
            if is_flags_changed && all_mods_held {
                KeyEvent::ComboPressed
            } else if is_flags_changed && !all_mods_held {
                KeyEvent::ComboReleased
            } else {
                KeyEvent::Other
            }
        }
    }
}

/// Pure push-to-talk state transition. Hold the combo to record (Start on press,
/// Stop on release); Escape while holding cancels. Matched events are swallowed
/// so the combo doesn't leak a keystroke to the focused app; held-key repeats are
/// suppressed.
pub fn classify(observed: KeyEvent, is_holding: bool) -> TapDecision {
    match observed {
        KeyEvent::ComboPressed if !is_holding => TapDecision {
            event: Some(HotkeyEvent::Start),
            holding: true,
            swallow: true,
        },
        KeyEvent::ComboPressed => TapDecision {
            event: None, // repeat-fire on held key
            holding: true,
            swallow: true,
        },
        KeyEvent::ComboReleased if is_holding => TapDecision {
            event: Some(HotkeyEvent::Stop),
            holding: false,
            swallow: true,
        },
        KeyEvent::EscapePressed if is_holding => TapDecision {
            event: Some(HotkeyEvent::Cancel),
            holding: false,
            swallow: true,
        },
        // Release/Escape while idle, or any other event: pass through untouched.
        KeyEvent::ComboReleased | KeyEvent::EscapePressed | KeyEvent::Other => TapDecision {
            event: None,
            holding: is_holding,
            swallow: false,
        },
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AudioCue {
    Start,
    Stop,
}

/// Global hotkey capture.
///
/// **Lifecycle:** call [`set_handler`](HotkeyCapture::set_handler) *before*
/// [`register`](HotkeyCapture::register) — events that fire before a handler is
/// set are swallowed but not delivered. `register` tears down any prior
/// registration first (re-registering rebinds the combo). `unregister` is
/// idempotent.
pub trait HotkeyCapture: Send + Sync {
    fn register(&self, combo: &HotkeyCombo) -> Result<(), HotkeyError>;
    fn unregister(&self) -> Result<(), HotkeyError>;
    fn set_handler(&self, handler: Box<dyn Fn(HotkeyEvent) + Send + Sync>);
    fn validate(&self, combo: &HotkeyCombo) -> Result<(), HotkeyError>;
    fn play_cue(&self, cue: AudioCue);
}

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_combo_round_trip() {
        let combo = HotkeyCombo {
            key: "Space".into(),
            modifiers: vec![Modifier::Option, Modifier::Shift],
        };
        let value = serde_json::to_value(&combo).expect("serialize");
        // camelCase struct fields; enum modifiers serialize snake_case.
        assert_eq!(value["key"], "Space");
        assert_eq!(value["modifiers"], serde_json::json!(["option", "shift"]));

        let back: HotkeyCombo = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.key, combo.key);
        assert_eq!(back.modifiers, combo.modifiers);
    }

    #[test]
    fn parse_combo_parses_modifiers_and_key() {
        let combo = parse_combo("Option+Shift+Space").expect("valid combo");
        assert_eq!(combo.key, "Space");
        assert_eq!(combo.modifiers, vec![Modifier::Option, Modifier::Shift]);
    }

    #[test]
    fn parse_combo_rejects_combo_without_modifier() {
        // A bare key with no modifier is too easy to trigger accidentally.
        assert!(matches!(
            parse_combo("Space"),
            Err(HotkeyError::InvalidCombo(_))
        ));
    }

    #[test]
    fn parse_combo_rejects_unknown_token() {
        assert!(matches!(
            parse_combo("Option+Nonsense"),
            Err(HotkeyError::InvalidCombo(_))
        ));
    }

    #[test]
    fn parse_combo_dedups_repeated_modifiers() {
        let combo = parse_combo("Option+Option+Space").unwrap();
        assert_eq!(combo.modifiers, vec![Modifier::Option]);
    }

    #[test]
    fn parse_combo_tolerates_trailing_and_whitespace() {
        let combo = parse_combo(" Option + Shift + Space + ").unwrap();
        assert_eq!(combo.key, "Space");
        assert_eq!(combo.modifiers, vec![Modifier::Option, Modifier::Shift]);
    }

    #[test]
    fn parse_combo_rejects_empty() {
        assert!(matches!(parse_combo(""), Err(HotkeyError::InvalidCombo(_))));
        assert!(matches!(
            parse_combo("+"),
            Err(HotkeyError::InvalidCombo(_))
        ));
    }

    #[test]
    fn check_conflicts_flags_spotlight() {
        let combo = parse_combo("Cmd+Space").unwrap();
        assert_eq!(
            check_conflicts(&combo),
            Err(HotkeyError::ConflictWith("Spotlight".to_string()))
        );
    }

    #[test]
    fn hotkey_error_code_and_display_cover_every_variant() {
        assert_eq!(HotkeyError::PermissionDenied.code(), "permission_denied");
        assert_eq!(
            HotkeyError::PermissionDenied.to_string(),
            "permission denied"
        );

        assert_eq!(
            HotkeyError::ConflictWith("Spotlight".to_string()).code(),
            "conflict_with:Spotlight"
        );
        assert_eq!(
            HotkeyError::ConflictWith("Spotlight".to_string()).to_string(),
            "conflicts with: Spotlight"
        );

        assert_eq!(
            HotkeyError::InvalidCombo("empty combo".to_string()).code(),
            "invalid_combo:empty combo"
        );
        assert_eq!(
            HotkeyError::InvalidCombo("empty combo".to_string()).to_string(),
            "invalid combo: empty combo"
        );

        assert_eq!(HotkeyError::TapCreationFailed.code(), "tap_creation_failed");
        assert_eq!(
            HotkeyError::TapCreationFailed.to_string(),
            "tap creation failed"
        );
    }

    #[test]
    fn check_conflicts_allows_unreserved_combo() {
        let combo = parse_combo("Option+Shift+Space").unwrap();
        assert!(check_conflicts(&combo).is_ok());
    }

    #[test]
    fn check_conflicts_is_modifier_order_independent() {
        // Ctrl+Up is Mission Control; the reversed/extra-typed combo still matches.
        let combo = HotkeyCombo {
            key: "Up".into(),
            modifiers: vec![Modifier::Ctrl],
        };
        assert!(matches!(
            check_conflicts(&combo),
            Err(HotkeyError::ConflictWith(_))
        ));
    }

    #[test]
    fn classify_press_while_idle_starts_and_swallows() {
        let d = classify(KeyEvent::ComboPressed, false);
        assert_eq!(d.event, Some(HotkeyEvent::Start));
        assert!(d.holding);
        assert!(d.swallow);
    }

    #[test]
    fn classify_press_while_holding_suppresses_repeat() {
        let d = classify(KeyEvent::ComboPressed, true);
        assert_eq!(d.event, None);
        assert!(d.holding);
        assert!(d.swallow);
    }

    #[test]
    fn classify_release_while_holding_stops() {
        let d = classify(KeyEvent::ComboReleased, true);
        assert_eq!(d.event, Some(HotkeyEvent::Stop));
        assert!(!d.holding);
        assert!(d.swallow);
    }

    #[test]
    fn classify_escape_while_holding_cancels() {
        let d = classify(KeyEvent::EscapePressed, true);
        assert_eq!(d.event, Some(HotkeyEvent::Cancel));
        assert!(!d.holding);
        assert!(d.swallow);
    }

    #[test]
    fn classify_escape_while_idle_passes_through() {
        let d = classify(KeyEvent::EscapePressed, false);
        assert_eq!(d.event, None);
        assert!(!d.holding);
        assert!(!d.swallow);
    }

    #[test]
    fn classify_other_event_is_noop() {
        let d = classify(KeyEvent::Other, true);
        assert_eq!(d.event, None);
        assert!(d.holding); // unchanged
        assert!(!d.swallow);
    }

    // observe(): map a raw key event (down/flags-changed, keycode, active
    // modifier mask) against the target combo into a KeyEvent.
    const TARGET_KEY: u16 = 49; // Space
    const TARGET_MASK: u64 = 0b11; // two arbitrary modifier bits
    const ESC: u16 = 53;

    #[test]
    fn observe_target_keydown_with_all_modifiers_is_combo_pressed() {
        let e = observe(
            true,
            false,
            TARGET_KEY,
            0b11,
            Some(TARGET_KEY),
            TARGET_MASK,
            ESC,
        );
        assert_eq!(e, KeyEvent::ComboPressed);
    }

    #[test]
    fn observe_target_keydown_missing_a_modifier_is_other() {
        let e = observe(
            true,
            false,
            TARGET_KEY,
            0b01,
            Some(TARGET_KEY),
            TARGET_MASK,
            ESC,
        );
        assert_eq!(e, KeyEvent::Other);
    }

    #[test]
    fn observe_target_keyup_is_combo_released() {
        let e = observe(
            false,
            false,
            TARGET_KEY,
            0b11,
            Some(TARGET_KEY),
            TARGET_MASK,
            ESC,
        );
        assert_eq!(e, KeyEvent::ComboReleased);
    }

    #[test]
    fn observe_modifier_dropped_via_flags_changed_is_combo_released() {
        // flagsChanged where a required modifier is no longer held.
        let e = observe(false, true, 0, 0b01, Some(TARGET_KEY), TARGET_MASK, ESC);
        assert_eq!(e, KeyEvent::ComboReleased);
    }

    #[test]
    fn observe_escape_keydown_is_escape_pressed() {
        let e = observe(true, false, ESC, 0, Some(TARGET_KEY), TARGET_MASK, ESC);
        assert_eq!(e, KeyEvent::EscapePressed);
    }

    #[test]
    fn observe_autorepeat_keydown_is_combo_pressed_again() {
        // macOS auto-repeat sends repeated keyDowns; observe still reports
        // ComboPressed (classify suppresses the repeat via is_holding).
        let e = observe(
            true,
            false,
            TARGET_KEY,
            0b11,
            Some(TARGET_KEY),
            TARGET_MASK,
            ESC,
        );
        assert_eq!(e, KeyEvent::ComboPressed);
        let d = classify(e, true);
        assert_eq!(d.event, None);
        assert!(d.holding);
        assert!(d.swallow);
    }

    #[test]
    fn observe_flags_changed_with_all_mods_still_held_is_other() {
        // A flagsChanged while every target modifier is still down must NOT be
        // treated as a release.
        let e = observe(false, true, 0, 0b11, Some(TARGET_KEY), TARGET_MASK, ESC);
        assert_eq!(e, KeyEvent::Other);
    }

    #[test]
    fn observe_flags_changed_adding_nontarget_modifier_is_other() {
        // Extra (non-target) modifier bits set, all target mods still held.
        let e = observe(
            false,
            true,
            0,
            0b11 | (1 << 40),
            Some(TARGET_KEY),
            TARGET_MASK,
            ESC,
        );
        assert_eq!(e, KeyEvent::Other);
    }

    // ---- modifier-only combos (target_keycode == None) ----

    #[test]
    fn parse_combo_parses_single_modifier_only() {
        let combo = parse_combo("Option").expect("valid modifier-only combo");
        assert!(combo.is_modifier_only());
        assert_eq!(combo.key, "");
        assert_eq!(combo.modifiers, vec![Modifier::Option]);
    }

    #[test]
    fn parse_combo_parses_multi_modifier_only() {
        let combo = parse_combo("Option+Shift").expect("valid modifier-only combo");
        assert!(combo.is_modifier_only());
        assert_eq!(combo.modifiers, vec![Modifier::Option, Modifier::Shift]);
    }

    #[test]
    fn parse_combo_keyed_is_not_modifier_only() {
        let combo = parse_combo("Option+Space").unwrap();
        assert!(!combo.is_modifier_only());
        assert_eq!(combo.key, "Space");
    }

    #[test]
    fn observe_modifier_only_flags_changed_all_held_is_combo_pressed() {
        let e = observe(false, true, 0, 0b11, None, TARGET_MASK, ESC);
        assert_eq!(e, KeyEvent::ComboPressed);
    }

    #[test]
    fn observe_modifier_only_flags_changed_released_is_combo_released() {
        let e = observe(false, true, 0, 0b01, None, TARGET_MASK, ESC);
        assert_eq!(e, KeyEvent::ComboReleased);
    }

    #[test]
    fn observe_modifier_only_escape_keydown_cancels() {
        let e = observe(true, false, ESC, 0b11, None, TARGET_MASK, ESC);
        assert_eq!(e, KeyEvent::EscapePressed);
    }

    #[test]
    fn observe_modifier_only_other_keydown_passes_through() {
        // A normal key pressed while the modifier is held is not the trigger.
        let e = observe(true, false, TARGET_KEY, 0b11, None, TARGET_MASK, ESC);
        assert_eq!(e, KeyEvent::Other);
    }
}

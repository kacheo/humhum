//! Native-menu-owned dictation availability. All shortcut registration shares
//! the control lock; recording startup uses try_lock to avoid event-tap joins
//! deadlocking on a callback waiting for the operation doing the join.
use std::sync::atomic::Ordering;

use crate::error::AppError;
use crate::prefs::{self, Prefs};
use crate::state::{AppState, RecordingPhase};

pub fn can_toggle(state: &AppState) -> bool {
    matches!(
        state.phase(),
        RecordingPhase::Idle | RecordingPhase::Done | RecordingPhase::Error
    )
}

/// Caller holds dictation_control (except setup before handlers are installed).
pub fn apply_hotkey(state: &AppState, prefs: &Prefs) -> Result<(), AppError> {
    if prefs.dictation_enabled {
        state
            .hotkey
            .register(&crate::hotkey::parse_combo(&prefs.hotkey)?)?;
    } else {
        state.hotkey.unregister()?;
    }
    Ok(())
}

/// Used at startup and after permission recovery. Never overrides user intent.
pub fn refresh_hotkey(state: &AppState) -> Result<(), AppError> {
    let _control = state.dictation_control.lock().unwrap();
    let prefs = state.cleanup_prefs.lock().unwrap().clone();
    apply_hotkey(state, &prefs)
}

pub fn set_enabled(state: &AppState, enabled: bool) -> Result<(), AppError> {
    set_enabled_with(state, enabled, prefs::save_prefs)
}

fn set_enabled_with(
    state: &AppState,
    enabled: bool,
    persist: impl FnOnce(&Prefs) -> Result<(), prefs::PrefsError>,
) -> Result<(), AppError> {
    let _control = state.dictation_control.lock().unwrap();
    if !can_toggle(state) {
        return Err(AppError::AlreadyRecording);
    }
    // Do not hold the preferences lock across native tap teardown: a main-
    // thread credential command can read preferences while a callback needs UI.
    let saved = state.cleanup_prefs.lock().unwrap().clone();
    if saved.dictation_enabled == enabled {
        return Ok(());
    }
    let mut next = saved.clone();
    next.dictation_enabled = enabled;
    // Touch the native shortcut before committing intent. On any failure,
    // restore the prior registration and leave both persisted and live intent.
    let result = apply_hotkey(state, &next).and_then(|()| persist(&next).map_err(AppError::from));
    if let Err(error) = result {
        if let Err(restore) = apply_hotkey(state, &saved) {
            log::error!("could not restore shortcut after failed toggle: {restore}");
        }
        return Err(error);
    }
    *state.cleanup_prefs.lock().unwrap() = next;
    state.dictation_enabled.store(enabled, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::{AudioCue, HotkeyCapture, HotkeyCombo, HotkeyError, HotkeyEvent};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Shortcut {
        registered: AtomicBool,
        fail_next: AtomicBool,
        combo: Mutex<String>,
    }
    impl HotkeyCapture for Shortcut {
        fn register(&self, combo: &HotkeyCombo) -> Result<(), HotkeyError> {
            self.registered.store(false, Ordering::SeqCst);
            if self.fail_next.swap(false, Ordering::SeqCst) {
                return Err(HotkeyError::TapCreationFailed);
            }
            *self.combo.lock().unwrap() = combo.key.clone();
            self.registered.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn unregister(&self) -> Result<(), HotkeyError> {
            self.registered.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn set_handler(&self, _: Box<dyn Fn(HotkeyEvent) + Send + Sync>) {}
        fn validate(&self, _: &HotkeyCombo) -> Result<(), HotkeyError> {
            Ok(())
        }
        fn play_cue(&self, _: AudioCue) {}
    }

    fn harness() -> (AppState, Arc<Shortcut>) {
        let shortcut = Arc::new(Shortcut::default());
        let mut state = AppState::new();
        state.hotkey = shortcut.clone();
        refresh_hotkey(&state).unwrap();
        (state, shortcut)
    }

    #[test]
    fn disable_releases_shortcut_and_enable_uses_current_combo() {
        let (state, shortcut) = harness();
        set_enabled_with(&state, false, |p| {
            assert!(!p.dictation_enabled);
            Ok(())
        })
        .unwrap();
        assert!(!shortcut.registered.load(Ordering::SeqCst));
        assert!(!state.dictation_enabled.load(Ordering::SeqCst));
        state.cleanup_prefs.lock().unwrap().hotkey = "Option+Return".into();
        // Preference changes and permission recovery use this same path.
        refresh_hotkey(&state).unwrap();
        assert!(!shortcut.registered.load(Ordering::SeqCst));
        set_enabled_with(&state, true, |p| {
            assert!(p.dictation_enabled);
            Ok(())
        })
        .unwrap();
        assert!(shortcut.registered.load(Ordering::SeqCst));
        assert_eq!(*shortcut.combo.lock().unwrap(), "Return");
    }

    #[test]
    fn disabled_startup_does_not_register() {
        let (state, shortcut) = harness();
        state.cleanup_prefs.lock().unwrap().dictation_enabled = false;
        state.dictation_enabled.store(false, Ordering::SeqCst);
        refresh_hotkey(&state).unwrap();
        assert!(!shortcut.registered.load(Ordering::SeqCst));
    }

    #[test]
    fn busy_toggle_is_rejected_without_persistence_or_shortcut_changes() {
        let (state, shortcut) = harness();
        for phase in [
            RecordingPhase::Recording,
            RecordingPhase::Transcribing,
            RecordingPhase::Cleaning,
            RecordingPhase::Pasting,
        ] {
            state.set_phase(phase);
            assert_eq!(
                set_enabled_with(&state, false, |_| panic!("must not persist"))
                    .unwrap_err()
                    .code(),
                "already_recording"
            );
            assert!(shortcut.registered.load(Ordering::SeqCst));
            assert!(state.dictation_enabled.load(Ordering::SeqCst));
        }
        for phase in [
            RecordingPhase::Idle,
            RecordingPhase::Done,
            RecordingPhase::Error,
        ] {
            state.set_phase(phase);
            set_enabled_with(&state, false, |_| Ok(())).unwrap();
            set_enabled_with(&state, true, |_| Ok(())).unwrap();
        }
    }

    #[test]
    fn failed_persistence_restores_previous_state_in_both_directions() {
        let (state, shortcut) = harness();
        let fail = |_: &Prefs| Err(crate::prefs::PrefsError::IoError("disk full".into()));
        assert!(set_enabled_with(&state, false, fail).is_err());
        assert!(shortcut.registered.load(Ordering::SeqCst));
        assert!(state.dictation_enabled.load(Ordering::SeqCst));
        assert!(state.cleanup_prefs.lock().unwrap().dictation_enabled);
        set_enabled_with(&state, false, |_| Ok(())).unwrap();
        assert!(set_enabled_with(&state, true, fail).is_err());
        assert!(!shortcut.registered.load(Ordering::SeqCst));
        assert!(!state.dictation_enabled.load(Ordering::SeqCst));
        assert!(!state.cleanup_prefs.lock().unwrap().dictation_enabled);
    }

    #[test]
    fn failed_enable_does_not_persist_or_report_enabled() {
        let (state, shortcut) = harness();
        set_enabled_with(&state, false, |_| Ok(())).unwrap();
        shortcut.fail_next.store(true, Ordering::SeqCst);
        assert!(set_enabled_with(&state, true, |_| panic!("must not persist")).is_err());
        assert!(!shortcut.registered.load(Ordering::SeqCst));
        assert!(!state.dictation_enabled.load(Ordering::SeqCst));
    }
}

pub mod audio;
pub mod cleanup;
pub mod commands;
mod dictation;
pub mod error;
pub mod events;
pub mod hotkey;
mod logging;
mod memory_diagnostics;
pub mod models;
pub mod paste;
pub mod permissions;
pub mod pipeline;
pub mod platform;
pub mod prefs;
mod profile;
pub mod recording;
pub mod state;
#[cfg(test)]
mod test_support;
pub mod transcription;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Listener, Manager};

fn refresh_dictation_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    item: &MenuItem<R>,
    pending: &AtomicBool,
) {
    let state = app.state::<state::AppState>();
    let enabled = state.dictation_enabled.load(Ordering::SeqCst);
    let _ = item.set_text(if enabled { "Disable" } else { "Enable" });
    let _ = item.set_enabled(!pending.load(Ordering::SeqCst) && dictation::can_toggle(&state));
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(if enabled {
            profile::APP_NAME
        } else {
            "Dictation disabled"
        }));
    }
}

/// Show and focus the preferences window (the only entry point once the app
/// lives in the menu bar). The window is normally kept alive and merely
/// hidden on close (see the `CloseRequested` handler in `run`), but if it is
/// ever missing — destroyed by an unexpected path — recreate it so Preferences
/// can always be reopened rather than silently no-op'ing.
fn show_prefs_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let w = match app.get_webview_window("prefs") {
        Some(w) => w,
        None => {
            // Mirror the `prefs` window config in tauri.conf.json — keep these
            // two definitions in lockstep (label/url/title/size/resizable).
            match tauri::WebviewWindowBuilder::new(
                app,
                "prefs",
                tauri::WebviewUrl::App("prefs.html".into()),
            )
            .title(format!("{} Preferences", profile::APP_NAME))
            .inner_size(500.0, 560.0)
            .resizable(false)
            .center()
            .visible(false)
            .build()
            {
                Ok(w) => w,
                Err(e) => {
                    log::error!("failed to recreate prefs window: {e}");
                    return;
                }
            }
        }
    };
    let _ = w.show();
    let _ = w.set_focus();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    profile::validate_identifier(&context.config().identifier).expect("application identity");
    let builder = tauri::Builder::default().manage(state::AppState::new());
    // Embedded WebDriver server for the WDIO E2E suite under `e2e/` — compiled
    // in only under the `e2e-harness` feature, which is absent from every real
    // build script (see `Cargo.toml`).
    #[cfg(feature = "e2e-harness")]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());
    // Opens the source and license link in the system browser.
    let builder = builder.plugin(tauri_plugin_opener::init());
    builder
        .on_window_event(|window, event| {
            // Our static windows are declared once at startup and reused via
            // show()/hide(). A user close (red button or ⌘W) would DESTROY the
            // window by default, after which it can never be shown again — hide
            // instead so the handle stays alive. Scoped by label so any future
            // genuinely-closable window still closes normally. App quit is
            // unaffected: the tray Quit item goes through NSApplication.terminate,
            // not this per-window CloseRequested event.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if matches!(window.label(), "prefs" | "overlay") {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .setup(|app| {
            // Menu-bar app: no Dock icon, accessory activation. LSUIElement in
            // Info.plist already implies this; set it explicitly so the policy
            // is unambiguous and windows still focus when shown.
            #[cfg(target_os = "macos")]
            let _ = app.handle().set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Log in release too (writes to ~/Library/Logs/com.kacheo.humhum/
            // Humhum.log) so shipping builds surface permission/hotkey
            // problems for support. Info level; the high-frequency per-keystroke
            // tap trace is compiled out of release (see hotkey/macos.rs).
            app.handle().plugin(
                logging::builder().build(),
            )?;

            // Startup permission gate (LLD §9.4): check the three macOS
            // permissions and warn if any are missing. No hard block — the
            // blocking UI is the Phase 20 onboarding flow. The
            // `permissions-changed` emit here is best-effort (no JS listener is
            // registered yet at setup time); the authoritative path for the
            // frontend is the `check_permissions` command on demand.
            let perms = permissions::create().check();
            if !perms.all_granted() {
                log::warn!(
                    "missing permissions at startup: microphone={} accessibility={} input_monitoring={}",
                    perms.microphone,
                    perms.accessibility,
                    perms.input_monitoring
                );
            }
            app.handle().emit("permissions-changed", perms)?;

            // Recording wiring (LLD §717/§719/§721): route hotkey events through
            // the recording controller — one code path shared with the IPC
            // commands. The handler fires on the CGEventTap thread;
            // `recording::*` uses `tauri::async_runtime`, callable from any thread.
            let handle = app.handle().clone();
            let app_state = app.state::<state::AppState>();
            memory_diagnostics::start(app_state.recording_phase.clone());
            app_state.hotkey.set_handler(Box::new(move |event| {
                let app = handle.clone();
                let st = app.state::<state::AppState>();
                // TEMP diagnostic (PR #34): confirm the tap→handler boundary fires.
                log::info!("hotkey handler: received {event:?}");
                let result = match event {
                    hotkey::HotkeyEvent::Start => recording::start(&st, &app),
                    hotkey::HotkeyEvent::Stop => recording::stop(&st, &app),
                    hotkey::HotkeyEvent::Cancel => recording::cancel(&st, &app),
                };
                if let Err(e) = result {
                    log::warn!("recording control failed: {}", e.code());
                }
            }));
            // Seed canonical state before the first shortcut registration.
            let initial_prefs = prefs::load_prefs();
            let _ = cleanup::usage::load(None);
            let config = cleanup::build_cleanup_config(&initial_prefs, |p| {
                prefs::read_api_key(p).ok().flatten()
            });
            app_state.dictation_enabled.store(initial_prefs.dictation_enabled, Ordering::SeqCst);
            *app_state.cleanup.write().unwrap() = config;
            *app_state.cleanup_prefs.lock().unwrap() = initial_prefs;
            if let Err(e) = dictation::refresh_hotkey(&app_state) {
                log::warn!("hotkey registration failed: {e}");
            }

            // Seed the preferred input device (Bluetooth-avoidance fix) from
            // prefs; `set_prefs` propagates any later change.
            app_state
                .audio
                .set_preferred_device(prefs::load_prefs().preferred_input_device.clone());

            // Startup model integrity check (LLD / bead 5na.6): hash base.en if
            // present and drop it if corrupt, so onboarding re-prompts. Hashing
            // ~148 MB must not block the setup thread, so run it off-pool.
            tauri::async_runtime::spawn_blocking(models::verify_base_model_on_startup);

            // System tray (LLD §12): the menu-bar presence + the only way to
            // reopen Preferences or quit once windows are closed.
            let prefs_item = MenuItem::with_id(app, "preferences", "Preferences…", true, None::<&str>)?;
            let quit_item = PredefinedMenuItem::quit(app, Some(&format!("Quit {}", profile::APP_NAME)))?;
            let toggle_item = MenuItem::with_id(app, "toggle-dictation", "Disable", true, None::<&str>)?;
            let toggle_error = MenuItem::with_id(app, "dictation-error", "", false, None::<&str>)?;
            let toggle_pending = Arc::new(AtomicBool::new(false));
            let menu = Menu::with_items(app, &[&toggle_item, &prefs_item, &quit_item])?;
            let action_menu = menu.clone();
            let action_toggle = toggle_item.clone();
            let action_pending = toggle_pending.clone();
            let tray = TrayIconBuilder::with_id("main");
            // Platform tray icon. On macOS the menu bar expects a monochrome
            // "template" glyph that the system auto-tints (white on dark menu
            // bars, dark on light) like every other icon — so use a black+alpha
            // glyph of just the bars and flag it as a template. `icon_as_template`
            // is macOS-only (a silent no-op elsewhere), and a black glyph would be
            // near-invisible on a Windows/Linux tray, so off macOS keep the
            // full-color app icon instead.
            #[cfg(target_os = "macos")]
            let tray = tray
                .icon(tauri::include_image!("icons/tray-template.png"))
                .icon_as_template(true);
            #[cfg(not(target_os = "macos"))]
            let tray =
                tray.icon(app.default_window_icon().expect("bundled default icon").clone());
            let tray = tray
                .tooltip(profile::APP_NAME)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, event| {
                    if event.id.as_ref() == "preferences" {
                        show_prefs_window(app);
                    } else if event.id.as_ref() == "toggle-dictation"
                        && !action_pending.swap(true, Ordering::SeqCst)
                    {
                        let _ = action_toggle.set_enabled(false);
                        let app = app.clone();
                        let menu = action_menu.clone();
                        let item = action_toggle.clone();
                        let error_item = toggle_error.clone();
                        let pending = action_pending.clone();
                        // Never join the event-tap thread from the main thread:
                        // its recording listeners may be updating native UI.
                        tauri::async_runtime::spawn_blocking(move || {
                            let state = app.state::<state::AppState>();
                            let enabled = !state.dictation_enabled.load(Ordering::SeqCst);
                            match dictation::set_enabled(&state, enabled) {
                                Ok(()) => {
                                    if menu.get("dictation-error").is_some() {
                                        let _ = menu.remove(&error_item);
                                    }
                                }
                                Err(e) => {
                                    log::warn!("dictation toggle failed: {e}");
                                    let message = if matches!(e, error::AppError::AlreadyRecording) {
                                        "Dictation is busy — try again when finished"
                                    } else if enabled {
                                        "Could not enable — try again"
                                    } else {
                                        "Could not disable — try again"
                                    };
                                    let _ = error_item.set_text(message);
                                    if menu.get("dictation-error").is_none() {
                                        let _ = menu.append(&error_item);
                                    }
                                }
                            }
                            pending.store(false, Ordering::SeqCst);
                            refresh_dictation_menu(&app, &item, &pending);
                        });
                    }
                })
                .build(app)?;

            // Recording indicator: a red dot beside the tray icon while a
            // capture is live; cleared on stop/cancel. (Transcription/paste run
            // after `recording-stopped`, so the dot tracks capture, not the
            // whole pipeline — matches the chosen UX.)
            //
            // Clear with `set_title(Some(""))`, NOT `set_title(None)`: on macOS
            // `tray-icon`'s `set_title` only calls the button's `setTitle` inside
            // its `Some(..)` branch, so passing `None` is a no-op and would leave
            // the `●` stuck. `TrayIcon::set_title` already hops to the main thread
            // internally (Tauri's `run_item_main_thread!`), so these listeners can
            // call it directly from the emitting (off-main) thread.
            refresh_dictation_menu(app.handle(), &toggle_item, &toggle_pending);
            let tray_rec = tray.clone();
            let recording_app = app.handle().clone();
            let recording_toggle = toggle_item.clone();
            let recording_pending = toggle_pending.clone();
            app.listen_any("recording-started", move |_| {
                let _ = tray_rec.set_title(Some("●"));
                refresh_dictation_menu(&recording_app, &recording_toggle, &recording_pending);
            });
            let tray_done = tray.clone();
            app.listen_any("recording-stopped", move |_| {
                let _ = tray_done.set_title(Some(""));
            });
            let tray_cancel = tray.clone();
            app.listen_any("recording-cancelled", move |_| {
                let _ = tray_cancel.set_title(Some(""));
            });

            // Permissions monitor: poll every 2s and emit `permissions-changed`
            // only when the grant set actually changes. Reuses the managed
            // `AppState.permissions` handle rather than constructing a second one.
            let mon = app.handle().clone();
            let mon_perms = app_state.permissions.clone();
            let monitor_toggle = toggle_item.clone();
            let monitor_pending = toggle_pending.clone();
            tauri::async_runtime::spawn(async move {
                let mut last = mon_perms.check();
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    refresh_dictation_menu(&mon, &monitor_toggle, &monitor_pending);
                    let now = mon_perms.check();
                    if now != last {
                        // The global hotkey tap needs BOTH Accessibility and Input
                        // Monitoring. When the pair becomes fully granted having been
                        // incomplete — first grant, or a re-grant after a mid-session
                        // revocation — re-register so the shortcut works again without
                        // an app restart. The handler set at startup is retained.
                        let tap_ready = now.accessibility && now.input_monitoring;
                        let tap_was_ready = last.accessibility && last.input_monitoring;
                        if tap_ready && !tap_was_ready {
                            let st = mon.state::<state::AppState>();
                            if let Err(e) = dictation::refresh_hotkey(&st) {
                                log::warn!("hotkey refresh after permission grant failed: {e}");
                            }
                        }
                        // A required grant was revoked mid-session — surface it for
                        // support logs. The `permissions-changed` emit below already
                        // drives the live Prefs notice so the user can re-grant; an
                        // in-flight recording resets via the capture error path
                        // (`recording::finish_capture_err`).
                        if last.microphone && !now.microphone {
                            log::warn!("microphone permission revoked mid-session");
                        }
                        if last.accessibility && !now.accessibility {
                            log::warn!(
                                "accessibility permission revoked mid-session — the global hotkey \
                                 will stop working until re-granted"
                            );
                        }
                        if last.input_monitoring && !now.input_monitoring {
                            log::warn!(
                                "input monitoring permission revoked mid-session — the global hotkey \
                                 will stop working until re-granted"
                            );
                        }
                        last = now;
                        let _ = mon.emit("permissions-changed", now);
                    }
                }
            });


            // First run: surface the onboarding window (it renders Onboarding
            // until prefs.onboardingComplete). Afterwards the app stays in the
            // tray and Preferences opens on demand.
            if !prefs::load_prefs().onboarding_complete {
                show_prefs_window(&app.handle());
            }

            // E2E-only: the WDIO suite under `e2e/` has no tray to click and no
            // onboarding gate to walk once onboardingComplete is seeded, so this
            // is its only way to reach `PrefsPanel` for the model-download,
            // a release build can never be redirected regardless of environment.
            #[cfg(debug_assertions)]
            if std::env::var("HUMHUM_E2E_SHOW_PREFS_ON_STARTUP").as_deref() == Ok("1") {
                show_prefs_window(&app.handle());
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_recording,
            commands::stop_recording,
            commands::cancel_recording,
            commands::get_prefs,
            commands::set_prefs,
            commands::validate_hotkey,
            commands::download_model,
            commands::get_model_status,
            commands::store_api_key,
            commands::test_api_key,
            commands::get_cleanup_models,
            commands::get_cleanup_usage,
            commands::has_api_key,
            commands::remove_api_key,
            commands::test_ollama,
            commands::check_permissions,
            commands::request_microphone,
            commands::open_accessibility_settings,
            commands::open_input_monitoring_settings,
            commands::list_input_devices,
        ])
        .run(context)
        .expect("error while running tauri application");
}

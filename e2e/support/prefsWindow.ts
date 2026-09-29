// Window handles ARE Tauri window labels (confirmed by the smoke spike) — no
// title/URL disambiguation needed, unlike a typical multi-window WebDriver app.
// Use the service's own `browser.tauri.switchWindow()` (not the raw W3C
// `browser.switchToWindow()`) even though both work identically for the
// embedded provider: switchWindow() also marks the session as
// user-switched, which suppresses the service's per-command
// "ensureActiveWindowFocus" auto-focus check (see beforeCommand in
// @wdio/tauri-service). Without that, every findElement/click/getTitle call
// re-probes all window states and can take ~5s each, multiplying into a
// spec-wide timeout on anything but a trivial test. `browser.tauri.*` is
// added unconditionally by the service (not the execute()/mocking surface
// that needs tauri-plugin-wdio + @wdio/tauri-plugin), so this doesn't
// reintroduce the dependency this harness deliberately avoided.
export async function switchToPrefs(): Promise<void> {
  await browser.tauri.switchWindow("prefs");
}

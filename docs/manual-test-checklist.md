# macOS release smoke test

Run these checks on a signed Humhum build before publishing:

- [ ] Fresh install opens onboarding and asks for Microphone, Accessibility, and Input Monitoring access.
- [ ] Default and custom hotkeys start, stop, and cancel dictation in another app.
- [ ] Base and Small English models download, verify, and transcribe; Small is available without activation.
- [ ] A recording continues past 60 seconds and auto-stops at the five-minute safety limit.
- [ ] Raw transcription pastes into the focused field; clipboard fallback works when paste is unavailable.
- [ ] Local Ollama cleanup and cloud cleanup using a newly entered API key work when configured.
- [ ] Quitting and reopening preserves Humhum preferences and Keychain keys.
- [ ] Source and license opens the Humhum GitHub repository.
- [ ] A second signed Humhum build with the same bundle ID and Team ID retains permission grants.

For Accessibility, test a fresh grant and recovery from an older app entry:

- Clicking **Open Settings** in onboarding or Preferences requests Accessibility for the running Humhum process, then opens the Accessibility pane. Polling on its own must not prompt.
- Enable the installed `/Applications/Humhum.app`, return to Humhum, and allow at least two seconds for the permission monitor. A native prompt is asynchronous; opening it does not count as a grant.
- If the app still shows **Needed**, fully quit using the Humhum menu-bar menu and reopen the installed app. Closing Preferences only hides the window.
- If Accessibility still shows **Needed** after a full restart, remove the old Humhum entry with the minus button in System Settings. Click **Open Settings** in the installed app again and enable Humhum. Check for development or DMG copies when selecting an app.
- Confirm recovery guidance disappears when both keyboard permissions are granted, and that the global shortcut and pasting work in another app.

The permission checks use macOS's effective authorization, not the appearance of a Settings toggle. A checked toggle alone does not justify treating Accessibility as granted. Apple documents [`AXIsProcessTrustedWithOptions`](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions) as a request for the current process; prompting is asynchronous and does not change the call's return value. [TN2206](https://developer.apple.com/library/archive/technotes/tn2206/) explains how code signing requirements identify an app across versions.

The API-key WebDriver test is currently excluded from the default E2E run because a WebDriver-spawned debug app may lack a default Keychain in the test session. Run that check manually until the runner behavior is confirmed.

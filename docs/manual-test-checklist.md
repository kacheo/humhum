# macOS release smoke test

Run these checks on a signed Wipzi build before publishing:

- [ ] Fresh install opens onboarding and asks for Microphone, Accessibility, and Input Monitoring access.
- [ ] Default and custom hotkeys start, stop, and cancel dictation in another app.
- [ ] Base and Small English models download, verify, and transcribe; Small is available without activation.
- [ ] A recording continues past 60 seconds and auto-stops at the five-minute safety limit.
- [ ] Raw transcription pastes into the focused field; clipboard fallback works when paste is unavailable.
- [ ] Local Ollama cleanup and cloud cleanup using a newly entered API key work when configured.
- [ ] Quitting and reopening preserves Wipzi preferences and Keychain keys.
- [ ] Source and license opens the Wipzi GitHub repository.
- [ ] A second signed Wipzi build with the same bundle ID and Team ID retains permission grants.

The API-key WebDriver test is currently excluded from the default E2E run because a WebDriver-spawned debug app may lack a default Keychain in the test session. Run that check manually until the runner behavior is confirmed.

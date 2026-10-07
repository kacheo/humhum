# Setup and troubleshooting

## Permissions and shortcuts

Use the installed `/Applications/Wipzi.app`, not a development build or the copy inside the DMG. In onboarding or Preferences, click **Open Settings** and grant Microphone, Accessibility, and Input Monitoring to that copy.

After changing a grant, return to Wipzi and allow at least two seconds. If it still shows **Needed**, quit from the menu bar and reopen. If that fails, remove the old Wipzi entry in System Settings, click **Open Settings** again, and grant access to the installed copy. See the [manual checklist](manual-test-checklist.md) for detailed recovery checks.

If a shortcut does not start recording, confirm permission status and try a different shortcut in Preferences. Quit other dictation apps that use the same shortcut. For paste failures, focus an editable field and check the clipboard fallback; some apps do not expose a usable focused text field.

## Models and audio

Base English downloads about 148 MB; Small English about 488 MB. Both transcribe English. Downloads need Internet access to Hugging Face. Wipzi verifies integrity before a model becomes usable; partial or corrupt downloads are not treated as ready. Check connectivity and disk space, then retry through Preferences.

If audio is missing or unclear, confirm Microphone permission and select the intended input device in Preferences. Bluetooth microphones may use a lower-quality call mode; try the Mac's built-in microphone.

After a model download completes, **None** cleanup mode does not require network access for transcription. Selecting cloud cleanup or a remote Ollama endpoint changes that.

## Local Ollama cleanup

Run Ollama locally, install the model selected in Wipzi (the default is `llama3`), and use its OpenAI-compatible endpoint, normally `http://localhost:11434/v1/`. Select Local Ollama, confirm the model name, click **Test**, and save Preferences. The test checks endpoint reachability and model availability. Cleanup requests have a timeout; when unavailable, Wipzi can fall back to the raw transcript.

## Cloud cleanup and Keychain

Select Cloud API and OpenAI or Anthropic, enter that provider's key, click **Test**, then save Preferences. A successful key test stores the key in macOS Keychain. An invalid key, unavailable provider, or Keychain error requires correction before relying on cloud cleanup. Verify billing and provider privacy settings in the provider's own account.

Wipzi does not currently offer an in-app key removal button. To remove keys, quit Wipzi, open Keychain Access, and delete only entries with service **`wipzi-api`** and account **`openai-key`** or **`anthropic-key`**. Do not delete unrelated entries. Selecting None disables cleanup without deleting stored keys.

## Uninstall and remove local data

Disable Launch at Login in Preferences, save, and quit Wipzi. Move `/Applications/Wipzi.app` to Trash. To remove settings and downloaded models too, move `~/Library/Application Support/wipzi` to Trash. Remove Keychain entries separately as described above; uninstalling does not remove them.

Diagnostic logs may exist in `~/Library/Logs/com.kacheo.wipzi`. Check and remove that folder if present and no longer needed. Review and redact logs before attaching them to an issue.

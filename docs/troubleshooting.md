# Setup and troubleshooting

## Permissions and shortcuts

For the installer beta, grant permissions to `/Applications/Humhum.app`, rather than a development build or the copy inside the DMG. If you build from source, grant permissions to the development build you actually run; rebuilding may require a new grant. In onboarding or Preferences, click **Open Settings** and grant Microphone, Accessibility, and Input Monitoring to that copy.

After changing a grant, return to Humhum and allow at least two seconds. If it still shows **Needed**, quit from the menu bar and reopen. If that fails, remove the old Humhum entry in System Settings, click **Open Settings** again, and grant access to the installed copy. See the [manual checklist](manual-test-checklist.md) for detailed recovery checks.

If a shortcut does not start recording, confirm permission status and try a different shortcut in Preferences. Quit other dictation apps that use the same shortcut. For paste failures, focus an editable field and check the clipboard fallback; some apps do not expose a usable focused text field.

## Models and audio

Base English downloads about 148 MB; Small English about 488 MB. Both transcribe English. Downloads need Internet access to Hugging Face. Humhum verifies integrity before a model becomes usable; partial or corrupt downloads are not treated as ready. Check connectivity and disk space, then retry through Preferences.

If audio is missing or unclear, confirm Microphone permission and select the intended input device in Preferences. Bluetooth microphones may use a lower-quality call mode; try the Mac's built-in microphone.

After a model download completes, **None** cleanup mode does not require network access for transcription. Selecting cloud cleanup or a remote Ollama endpoint changes that.

## Local Ollama cleanup

Run Ollama locally, install the model selected in Humhum (the default is `llama3`), and use its OpenAI-compatible endpoint, normally `http://localhost:11434/v1/`. Select Local Ollama, confirm the model name, click **Test**, and save Preferences. The test checks endpoint reachability and model availability. Cleanup requests have a timeout; when unavailable, Humhum can fall back to the raw transcript.

## Cloud cleanup and Keychain

1. Open **Preferences → AI → Text cleanup**, select **Cloud API**, and choose **OpenAI** or **Anthropic**. The same controls are available during onboarding.
2. Enter that provider's API key and click **Verify & Save**. Verification makes a small billable request using fixed sample text against the cleanup model: `gpt-4o-mini` for OpenAI or Claude Haiku 4.5 for Anthropic. OpenAI API billing is separate from a ChatGPT subscription.
3. Wait for **Key verified and saved**, then click **Save** to apply provider and cleanup-mode changes. During onboarding, these choices are applied before the Try it step.

Keys are stored separately per provider in macOS Keychain. **Key saved in Keychain** means a credential is stored, not that its current billing or access has been rechecked. The field stays empty after saving or reopening the app. To replace a key, enter a new one and click **Verify & Save**; an unsuccessful verification keeps the previous key. To delete the selected provider's key, click **Remove key**. Saving, replacing, and removing keys take effect immediately for subsequent dictations using the saved provider; **Discard** does not undo those actions. A dictation already processing may finish with its existing configuration.

- **Key rejected:** check the key and its permission to use the cleanup endpoint and model.
- **Rate limit or quota reached:** check API billing, credits, and usage limits, or retry later.
- **Could not reach the cleanup service or model:** check connectivity and provider availability. Verification times out after five seconds.
- **Unable to access Keychain:** unlock your login keychain, then use **Refresh status** or retry the action. This is distinct from **No key saved**.

Cloud cleanup sends transcript text, not audio. Verify billing and privacy settings in the provider's account. Selecting **None** disables cleanup without deleting keys. If Cloud API remains selected after key removal, dictation reports that no key is configured; select None and save to use raw local transcripts.

For manual removal after quitting Humhum, open Keychain Access and delete only entries with service **`humhum-api`** and account **`openai-key`** or **`anthropic-key`**.

## Uninstall and remove local data

Disable Launch at Login in Preferences, save, and quit Humhum. Move `/Applications/Humhum.app` to Trash. To remove settings and downloaded models too, move `~/Library/Application Support/humhum` to Trash. Remove Keychain entries separately as described above; uninstalling does not remove them.

Diagnostic logs may exist in `~/Library/Logs/com.kacheo.humhum`. Check and remove that folder if present and no longer needed. Review and redact logs before attaching them to an issue.

## Moving from Wipzi

Humhum starts fresh and does not import Wipzi settings, downloaded models, or API keys. Grant permissions to Humhum, download models again, and configure cleanup again if desired. Wipzi's data under `~/Library/Application Support/wipzi`, keys under service `wipzi-api`, app, and login registration are left untouched. Quit Wipzi and disable its Launch at Login setting before running Humhum, so both apps do not respond to the same shortcut.

The new bundle identifier is `com.kacheo.humhum`; prior Wipzi grants do not establish Humhum permission status. Unsigned development builds may need permissions re-granted after rebuilding.

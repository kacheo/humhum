import { useState } from "react";
import type { ApiProvider } from "../../lib/types";
import { ApiKeyInput } from "../ApiKeyInput";

/** Shared cloud setup. Provider selection is a preference; credential actions
 *  persist independently and immediately in Keychain. */
export function CloudApiSettings({ provider, onProviderChange, onboarding = false }: {
  provider: ApiProvider;
  onProviderChange: (provider: ApiProvider) => void;
  onboarding?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const row = onboarding ? "onb-row" : "prefs__row";
  const hint = onboarding ? "onb-hint" : "prefs__hint";
  return (
    <>
      <label className={row}>
        <span>Provider</span>
        <select value={provider} disabled={busy} onChange={(e) => onProviderChange(e.target.value as ApiProvider)}>
          <option value="open_ai">OpenAI</option>
          <option value="anthropic">Anthropic</option>
        </select>
      </label>
      <div className={`${row} ${row}--col`}>
        <span>API key</span>
        <ApiKeyInput provider={provider} onBusyChange={setBusy} />
        <p className={hint}>Verification makes a small billable API request using sample text. Cloud cleanup sends transcript text, not audio, to your provider.</p>
        <p className={hint}>Keys are saved or removed immediately in Keychain; Discard does not undo key actions. {onboarding ? "Your cleanup choice is applied as you continue setup." : "Save applies provider and cleanup-mode changes."}</p>
        <p className={hint}>Cleanup model: {provider === "open_ai" ? "gpt-4o-mini" : "Claude Haiku 4.5"}.</p>
      </div>
    </>
  );
}

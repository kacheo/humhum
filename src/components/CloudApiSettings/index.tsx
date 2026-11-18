import { useCallback, useEffect, useState } from "react";
import { ipc } from "../../lib/ipc";
import type { ApiProvider, CleanupModelOption } from "../../lib/types";
import { ApiKeyInput } from "../ApiKeyInput";

/** Shared cloud setup. Provider selection is a preference; credential actions
 *  persist independently and immediately in Keychain. */
export function CloudApiSettings({ provider, onProviderChange, onBusyChange, onboarding = false, model, onModelChange }: {
  provider: ApiProvider;
  onProviderChange: (provider: ApiProvider) => void;
  onBusyChange?: (busy: boolean) => void;
  onboarding?: boolean;
  model?: string;
  onModelChange?: (model: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [catalog, setCatalog] = useState<CleanupModelOption[]>([]);
  const [catalogError, setCatalogError] = useState(false);
  useEffect(() => {
    if (onboarding) return;
    let active = true;
    ipc.getCleanupModels().then(
      (options) => { if (active) setCatalog(options ?? []); },
      () => { if (active) setCatalogError(true); },
    );
    return () => { active = false; };
  }, [onboarding]);
  const options = catalog.filter((entry) => entry.provider === provider);
  const selected = options.find((entry) => entry.id === model)?.id ?? options[0]?.id;
  const handleBusyChange = useCallback((value: boolean) => {
    setBusy(value);
    onBusyChange?.(value);
  }, [onBusyChange]);
  const row = onboarding ? "onb-row" : "prefs__row";
  const hint = onboarding ? "onb-hint" : "prefs__hint";
  return (
    <>
      <div className={onboarding ? undefined : "cloud-model-fields"}>
        <label className={row}>
          <span>Provider</span>
          <select value={provider} disabled={busy} onChange={(e) => onProviderChange(e.target.value as ApiProvider)}>
            <option value="open_ai">OpenAI</option>
            <option value="anthropic">Anthropic</option>
          </select>
        </label>
        {!onboarding && (
          <label className={row}>
            <span>Cleanup model</span>
            <select value={selected ?? ""} disabled={busy || !selected} onChange={(e) => onModelChange?.(e.target.value)}>
              {!selected && <option value="">{catalogError ? "Models unavailable" : "Loading models…"}</option>}
              {options.map((entry) => <option key={entry.id} value={entry.id}>{entry.label}</option>)}
            </select>
          </label>
        )}
      </div>
      <div className={`${row} ${row}--col`}>
        <span>API key</span>
        <ApiKeyInput provider={provider} verificationDisabled={!onboarding && !selected} onBusyChange={handleBusyChange} onTest={model && selected ? (p, key) => ipc.testApiKey(p, key, selected) : undefined} />
        <p className={hint}>Verification sends a short, billable sample. During dictation, your provider receives the transcript.</p>
        <p className={hint}>Keychain changes take effect immediately. {onboarding ? "Continue to apply your cleanup settings." : "Click Save to apply cleanup settings."}</p>
        {onboarding && <p className={hint}>Cleanup model: {provider === "open_ai" ? "gpt-4o-mini" : "Claude Haiku 4.5"}.</p>}
      </div>
    </>
  );
}

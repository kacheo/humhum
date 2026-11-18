import { useCallback, useEffect, useId, useState } from "react";
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
  const [catalog, setCatalog] = useState<CleanupModelOption[] | null>(null);
  const verificationHintId = useId();
  const persistenceHintId = useId();
  useEffect(() => {
    if (onboarding) return;
    let active = true;
    ipc.getCleanupModels().then(
      (options) => { if (active) setCatalog(options ?? []); },
      () => { if (active) setCatalog([]); },
    );
    return () => { active = false; };
  }, [onboarding]);
  const options = catalog?.filter((entry) => entry.provider === provider) ?? [];
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
              {!selected && <option value="">{catalog ? "Models unavailable" : "Loading models…"}</option>}
              {options.map((entry) => <option key={entry.id} value={entry.id}>{entry.label}</option>)}
            </select>
          </label>
        )}
      </div>
      <div className={`${row} ${row}--col cloud-credentials`}>
        <span>API key</span>
        {onboarding && <p className={hint}>Default model {provider === "open_ai" ? "gpt-4o-mini" : "Claude Haiku 4.5"}.</p>}
        <p className={hint} id={verificationHintId}>
          Verification sends a billable sample. Cloud cleanup sends transcripts to your provider.
        </p>
        <ApiKeyInput
          provider={provider}
          verificationDisabled={!onboarding && !selected}
          onBusyChange={handleBusyChange}
          onTest={selected ? (p, key) => ipc.testApiKey(p, key, selected) : undefined}
          verificationDescription={verificationHintId}
          persistenceDescription={persistenceHintId}
        />
        <p className={hint} id={persistenceHintId}>
          Key changes apply immediately.
        </p>
      </div>
    </>
  );
}

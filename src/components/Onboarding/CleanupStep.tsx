import { useCallback, useEffect, useState } from "react";
import { usePrefsStore } from "../../stores/prefs";
import { CloudApiSettings } from "../CloudApiSettings";

/** Cleanup step. Off, Local (Ollama), and Cloud API are all selectable (Phase
 *  12); the cloud-API key is tested/stored via the ApiKeyInput below. A mode is
 *  always selected, so ready unless a key action is in flight. */
export function CleanupStep({ onReady, onBusyChange }: {
  onReady: (ready: boolean) => void;
  onBusyChange?: (busy: boolean) => void;
}) {
  const [busy, setBusy] = useState(false);
  const handleBusyChange = useCallback((value: boolean) => {
    setBusy(value);
    onBusyChange?.(value);
  }, [onBusyChange]);
  const prefs = usePrefsStore((s) => s.prefs);
  const setField = usePrefsStore.getState().setField;
  useEffect(() => { onReady(!busy); }, [onReady, busy]);

  return (
    <section className="onb-step">
      <h2>Cleanup</h2>
      <p>Optionally tidy up transcripts. You can change this anytime in Preferences.</p>
      <label className="onb-row">
        <input type="radio" name="cleanup" disabled={busy} checked={prefs.cleanupMode === "none"} onChange={() => setField("cleanupMode", "none")} />
        <span>Off</span>
      </label>
      <label className="onb-row">
        <input type="radio" name="cleanup" disabled={busy} checked={prefs.cleanupMode === "local"} onChange={() => setField("cleanupMode", "local")} />
        <span>Local (Ollama)</span>
      </label>
      <label className="onb-row">
        <input type="radio" name="cleanup" disabled={busy} checked={prefs.cleanupMode === "api"} onChange={() => setField("cleanupMode", "api")} aria-label="Cloud API" />
        <span>Cloud API</span>
      </label>
      {prefs.cleanupMode === "api" && (
        <p className="onb-hint">Requires an API key — enter it below.</p>
      )}
      {prefs.cleanupMode === "local" && (
        <>
          <label className="onb-row">
            <span>Ollama model</span>
            <input aria-label="Ollama model" value={prefs.ollamaModel} onChange={(e) => setField("ollamaModel", e.target.value)} />
          </label>
          <label className="onb-row">
            <span>Ollama URL</span>
            <input aria-label="Ollama URL" value={prefs.ollamaBaseUrl} onChange={(e) => setField("ollamaBaseUrl", e.target.value)} />
          </label>
        </>
      )}
      {prefs.cleanupMode === "api" && (
        <CloudApiSettings provider={prefs.apiProvider} onProviderChange={(p) => setField("apiProvider", p)} onBusyChange={handleBusyChange} onboarding />
      )}
    </section>
  );
}

import { useEffect } from "react";
import { usePrefsStore } from "../../stores/prefs";
import { ipc } from "../../lib/ipc";
import { ApiKeyInput } from "../ApiKeyInput";

/** Cleanup step. Off, Local (Ollama), and Cloud API are all selectable (Phase
 *  12); the cloud-API key is tested/stored via the ApiKeyInput below. A mode is
 *  always selected, so always ready. */
export function CleanupStep({ onReady }: { onReady: (ready: boolean) => void }) {
  const prefs = usePrefsStore((s) => s.prefs);
  const setField = usePrefsStore.getState().setField;
  useEffect(() => { onReady(true); }, [onReady]);

  return (
    <section className="onb-step">
      <h2>Cleanup</h2>
      <p>Optionally tidy up transcripts. You can change this anytime in Preferences.</p>
      <label className="onb-row">
        <input type="radio" name="cleanup" checked={prefs.cleanupMode === "none"} onChange={() => setField("cleanupMode", "none")} />
        <span>Off</span>
      </label>
      <label className="onb-row">
        <input type="radio" name="cleanup" checked={prefs.cleanupMode === "local"} onChange={() => setField("cleanupMode", "local")} />
        <span>Local (Ollama)</span>
      </label>
      <label className="onb-row">
        <input type="radio" name="cleanup" checked={prefs.cleanupMode === "api"} onChange={() => setField("cleanupMode", "api")} aria-label="Cloud API" />
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
      {prefs.cleanupMode === "api" && <div className="onb-row onb-row--col">
        <span>Cloud API key</span>
        <ApiKeyInput provider={prefs.apiProvider} onTest={(p, k) => ipc.testApiKey(p, k)} />
        <p className="onb-hint">Testing a valid key saves it immediately in Keychain. Cloud cleanup sends transcript text to your provider.</p>
      </div>}
    </section>
  );
}

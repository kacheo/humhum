import { useEffect } from "react";
import { usePrefsStore } from "../../stores/prefs";
import { useRecordingStore } from "../../stores/recording";
import { ipc } from "../../lib/ipc";
import type { CleanupAggressiveness, CleanupMode, ModelStatus, WhisperModel } from "../../lib/types";
import { ApiKeyInput } from "../ApiKeyInput";
import { OllamaTest } from "../OllamaTest";
import { ModelStatusBadge } from "../ModelStatusBadge";

const MODELS: { id: WhisperModel; label: string }[] = [
  { id: "base_en", label: "Base (English)" },
  { id: "small_en", label: "Small (English)" },
];

/** The AI tab: transcription model + text cleanup, unified under one
 *  grouping. Cleanup's backend-specific fields (Ollama vs. Cloud API) are
 *  gated symmetrically on `cleanupMode`, and Aggressiveness — which applies
 *  to either backend — is shown only once a backend is selected. */
export function AiSection() {
  const prefs = usePrefsStore((s) => s.prefs);
  const modelStatus = useRecordingStore((s) => s.modelStatus);
  const downloadProgress = useRecordingStore((s) => s.downloadProgress);
  const setField = usePrefsStore.getState().setField;

  useEffect(() => {
    for (const { id } of MODELS) {
      ipc
        .getModelStatus(id)
        .then((s) => useRecordingStore.getState().setModelStatus(id, s))
        .catch(() => {});
    }
  }, []);

  function effectiveStatus(id: WhisperModel): ModelStatus {
    const s = modelStatus[id];
    if (s === "ready" || (typeof s === "object" && "error" in s)) return s;
    const pct = downloadProgress[id];
    return pct > 0 ? { downloading: pct } : s;
  }

  return (
    <>
      <section className="prefs__section">
        <h2 className="prefs__heading">Transcription</h2>
        <label className="prefs__row">
          <span>Model</span>
          <select
            value={prefs.whisperModel}
            onChange={(e) => setField("whisperModel", e.target.value as WhisperModel)}
          >
            {MODELS.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label}
              </option>
            ))}
          </select>
        </label>
        {MODELS.map((m) => {
          const status = effectiveStatus(m.id);
          const downloading = typeof status === "object" && "downloading" in status;
          const failed = typeof status === "object" && "error" in status;
          return (
            <div key={m.id} className="prefs__model-row">
              <span className="prefs__row-label">{m.label}</span>
              <ModelStatusBadge status={status} />
              <button
                type="button"
                aria-label={`${failed ? "Retry" : "Download"} ${m.label}`}
                disabled={status === "ready" || downloading}
                onClick={() => {
                  useRecordingStore.getState().setDownloadProgress(m.id, 0);
                  useRecordingStore.getState().setModelStatus(m.id, { downloading: 0 });
                  void ipc.downloadModel(m.id).catch(() => {
                    useRecordingStore.getState().setModelStatus(m.id, { error: "Could not start the download. Try again." });
                  });
                }}
              >
                {status === "ready" ? "Downloaded" : failed ? "Retry" : "Download"}
              </button>
            </div>
          );
        })}
        <p className="prefs__hint">Downloads start immediately. Speech is transcribed on your Mac.</p>
      </section>

      <section className="prefs__section">
        <h2 className="prefs__heading">Text cleanup</h2>
        <label className="prefs__row">
          <span>Mode</span>
          <select
            value={prefs.cleanupMode}
            onChange={(e) => setField("cleanupMode", e.target.value as CleanupMode)}
          >
            <option value="none">None</option>
            <option value="local">Local (Ollama)</option>
            <option value="api">Cloud API</option>
          </select>
        </label>

        {prefs.cleanupMode === "local" && (
          <>
            <label className="prefs__row">
              <span>Ollama model</span>
              <input value={prefs.ollamaModel} onChange={(e) => setField("ollamaModel", e.target.value)} />
            </label>
            <label className="prefs__row">
              <span>Ollama URL</span>
              <input value={prefs.ollamaBaseUrl} onChange={(e) => setField("ollamaBaseUrl", e.target.value)} />
            </label>
            <div className="prefs__row">
              <span>Connection</span>
              <OllamaTest baseUrl={prefs.ollamaBaseUrl} model={prefs.ollamaModel} />
            </div>
          </>
        )}

        {prefs.cleanupMode === "api" && (
          <>
            <div className="prefs__row prefs__row--col">
              <span>API key</span>
              <ApiKeyInput provider={prefs.apiProvider} onTest={(p, k) => ipc.testApiKey(p, k)} />
              <p className="prefs__hint">Testing a valid key saves it immediately in Keychain. Cloud cleanup sends transcript text to your provider.</p>
            </div>
          </>
        )}

        {prefs.cleanupMode !== "none" && (
          <>
            <label className="prefs__row">
              <span>Aggressiveness</span>
              <select
                value={prefs.cleanupAggressiveness}
                onChange={(e) => setField("cleanupAggressiveness", e.target.value as CleanupAggressiveness)}
              >
                <option value="light">Light</option>
                <option value="standard">Standard</option>
                <option value="heavy">Heavy</option>
              </select>
            </label>
            <p className="prefs__hint">Applies to both Local and Cloud cleanup.</p>
          </>
        )}
      </section>
    </>
  );
}

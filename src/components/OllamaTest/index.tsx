import { ipc } from "../../lib/ipc";
import { useAsyncProbe } from "../../hooks/useAsyncProbe";
import { ProbeStatus } from "../ProbeStatus";
import "../../styles/tokens.css";
import "./index.css";

/** "Test" button for the local Ollama setup. Verifies the server is reachable
 *  AND the configured model is installed (test_ollama → verify_ollama). A failed
 *  invoke rejects with a flat code string ("model_not_found" / "unavailable"),
 *  which we map to friendly copy here. Nothing is stored — this is a pure probe. */
export function OllamaTest({ baseUrl, model }: { baseUrl: string; model: string }) {
  const { testing, result, run } = useAsyncProbe();

  async function handleTest() {
    await run(async () => {
      try {
        await ipc.testOllama(baseUrl, model);
        return `Ollama reachable — ${model} ready`;
      } catch (e) {
        // AppError serializes to a bare wire code, so the rejection is the code string.
        const code = e instanceof Error ? e.message : String(e);
        throw new Error(
          code === "model_not_found" ? `Model '${model}' not found on server` : `Can't reach Ollama at ${baseUrl}`,
        );
      }
    });
  }

  return (
    <div className="ollama-test">
      <button
        type="button"
        disabled={testing || baseUrl.length === 0 || model.length === 0}
        onClick={handleTest}
      >
        {testing ? "Testing…" : "Test"}
      </button>
      <ProbeStatus result={result} />
    </div>
  );
}

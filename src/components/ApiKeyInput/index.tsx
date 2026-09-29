import { useState } from "react";
import type { ApiProvider } from "../../lib/types";
import { useAsyncProbe } from "../../hooks/useAsyncProbe";
import { ProbeStatus } from "../ProbeStatus";
import "../../styles/tokens.css";
import "./index.css";

/** Cloud cleanup API key input + Test button. The key is held only in local
 *  component state and is never logged. onTest runs test_api_key (Phase 11);
 *  disabled until that lands. */
export function ApiKeyInput({
  provider,
  disabled = false,
  onTest,
}: {
  provider: ApiProvider;
  disabled?: boolean;
  onTest?: (provider: ApiProvider, key: string) => Promise<void>;
}) {
  const [key, setKey] = useState("");
  const { testing, result, run } = useAsyncProbe();

  async function handleTest() {
    if (!onTest) return;
    await run(async () => {
      await onTest(provider, key);
      return "Key verified";
    });
  }

  return (
    <div className="api-key-input">
      <input
        type="password"
        className="api-key-input__field"
        aria-label="API key"
        placeholder={`${provider === "open_ai" ? "OpenAI" : "Anthropic"} API key`}
        value={key}
        disabled={disabled}
        onChange={(e) => setKey(e.target.value)}
      />
      <button type="button" disabled={disabled || testing || key.length === 0} onClick={handleTest}>
        {testing ? "Testing…" : "Test"}
      </button>
      {disabled && <span className="api-key-input__note">Available in a later build</span>}
      <ProbeStatus result={result} />
    </div>
  );
}

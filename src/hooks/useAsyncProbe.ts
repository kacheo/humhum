import { useState } from "react";

export interface ProbeResult {
  ok: boolean;
  msg: string;
}

/** Shared "testing/result" state machine for probe-and-report widgets
 *  (ApiKeyInput, OllamaTest). `action` resolves to the success message, or
 *  throws an Error whose `message` is shown as the failure text. */
export function useAsyncProbe() {
  const [testing, setTesting] = useState(false);
  const [result, setResult] = useState<ProbeResult | null>(null);

  async function run(action: () => Promise<string>) {
    setTesting(true);
    setResult(null);
    try {
      const msg = await action();
      setResult({ ok: true, msg });
    } catch (e) {
      setResult({ ok: false, msg: e instanceof Error ? e.message : String(e) });
    } finally {
      setTesting(false);
    }
  }

  return { testing, result, run };
}

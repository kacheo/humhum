import { useEffect, useRef, useState } from "react";
import type { ApiProvider } from "../../lib/types";
import type { ProbeResult } from "../../hooks/useAsyncProbe";
import { ipc } from "../../lib/ipc";
import { ProbeStatus } from "../ProbeStatus";
import "../../styles/tokens.css";
import "./index.css";

interface ApiKeyInputProps {
  provider: ApiProvider;
  disabled?: boolean;
  onTest?: (provider: ApiProvider, key: string) => Promise<void>;
  onBusyChange?: (busy: boolean) => void;
}

const STATUS_TEXT = {
  checking: "Checking Keychain…",
  saved: "Key saved in Keychain",
  missing: "No key saved",
  error: "Unable to access Keychain",
};
const KEY_CHANGED_EVENT = "humhum-api-key-changed";

function keyError(error: unknown): string {
  const code = error instanceof Error ? error.message : String(error);
  switch (code) {
    case "key_rejected": return "Key rejected. Check the key and its access to the cleanup model.";
    case "rate_limited": return "Rate limit or quota reached. Check provider billing or try again later.";
    case "keychain_error": return "Could not access Keychain. Unlock your login keychain and try again.";
    case "io_error":
    case "serialize_error": return "Could not update cleanup settings. Check your configuration and try again.";
    case "unavailable": return "Could not reach the cleanup service or model. Check your connection and try again.";
    case "key_action_superseded": return "A newer key action replaced this request. Check the current saved-key status.";
    default: return "Could not complete the key action. Try again.";
  }
}

/** Remount on provider changes: neither secrets nor late results can leak into
 *  another provider's editor. Stored credentials never enter the webview. */
export function ApiKeyInput(props: ApiKeyInputProps) {
  return <ProviderKeyInput key={props.provider} {...props} />;
}

function ProviderKeyInput({
  provider,
  disabled = false,
  onTest = ipc.testApiKey,
  onBusyChange,
}: ApiKeyInputProps) {
  const [key, setKey] = useState("");
  const [status, setStatus] = useState<keyof typeof STATUS_TEXT>("checking");
  const [action, setAction] = useState<"saving" | "removing" | null>(null);
  const [result, setResult] = useState<ProbeResult | null>(null);
  const [refresh, setRefresh] = useState(0);
  const mounted = useRef(false);
  const operationInFlight = useRef(false);
  const needsRefresh = useRef(false);
  const ownChange = useRef<Event | null>(null);
  const lookupVersion = useRef(0);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; onBusyChange?.(false); };
  }, [onBusyChange]);

  // A request can finish after its editor was unmounted by a mode switch.
  // Notify the current editor with provider identity only, never key material.
  useEffect(() => {
    const changed = (event: Event) => {
      if ((event as CustomEvent<ApiProvider>).detail !== provider || event === ownChange.current) return;
      lookupVersion.current += 1;
      setResult((current) => current?.ok ? null : current);
      if (operationInFlight.current) needsRefresh.current = true;
      else setRefresh((n) => n + 1);
    };
    window.addEventListener(KEY_CHANGED_EVENT, changed);
    return () => window.removeEventListener(KEY_CHANGED_EVENT, changed);
  }, [provider]);

  useEffect(() => {
    let active = true;
    const version = ++lookupVersion.current;
    setStatus("checking");
    void ipc.hasApiKey(provider).then(
      (saved) => { if (active && version === lookupVersion.current) setStatus(saved ? "saved" : "missing"); },
      () => { if (active && version === lookupVersion.current) setStatus("error"); },
    );
    return () => { active = false; };
  }, [provider, refresh]);

  const busy = disabled || action !== null;
  const blocked = busy || status === "checking";

  async function run(next: "saving" | "removing") {
    if (blocked || operationInFlight.current) return;
    operationInFlight.current = true;
    setAction(next);
    onBusyChange?.(true);
    setResult(null);
    try {
      if (next === "saving") await onTest(provider, key.trim());
      else await ipc.removeApiKey(provider);
      ownChange.current = new CustomEvent<ApiProvider>(KEY_CHANGED_EVENT, { detail: provider });
      window.dispatchEvent(ownChange.current);
      ownChange.current = null;
      if (!mounted.current) return;
      setKey("");
      setStatus(next === "saving" ? "saved" : "missing");
      setResult({ ok: true, msg: next === "saving" ? "Key verified and saved" : "Key removed" });
    } catch (error) {
      if (mounted.current) {
        setResult({ ok: false, msg: keyError(error) });
        const code = error instanceof Error ? error.message : String(error);
        if (code === "key_action_superseded") needsRefresh.current = true;
      }
    } finally {
      operationInFlight.current = false;
      if (mounted.current) {
        if (needsRefresh.current) {
          needsRefresh.current = false;
          setResult((current) => current?.ok ? null : current);
          setRefresh((n) => n + 1);
        }
        setAction(null);
        onBusyChange?.(false);
      }
    }
  }

  return (
    <div className="api-key-input">
      <input
        type="password"
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        className="api-key-input__field"
        aria-label="API key"
        placeholder={`${provider === "open_ai" ? "OpenAI" : "Anthropic"} API key`}
        value={key}
        disabled={busy}
        onChange={(e) => { setKey(e.target.value); setResult(null); }}
      />
      <button type="button" disabled={blocked || key.trim().length === 0} onClick={() => void run("saving")}>
        {action === "saving" ? "Verifying…" : "Verify & Save"}
      </button>
      <span className="api-key-input__note" aria-live="polite">{STATUS_TEXT[status]}</span>
      {(status === "saved" || status === "error") && (
        <button type="button" disabled={blocked} onClick={() => void run("removing")}>
          {action === "removing" ? "Removing…" : "Remove key"}
        </button>
      )}
      {status === "error" && (
        <button type="button" disabled={blocked} onClick={() => setRefresh((n) => n + 1)}>Refresh status</button>
      )}
      <ProbeStatus result={result} />
    </div>
  );
}

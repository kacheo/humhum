import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ipc } from "../../lib/ipc";
import type { CleanupUsage } from "../../lib/types";

const providerNames = { open_ai: "OpenAI", anthropic: "Anthropic", ollama: "Ollama" };

function format(count: number | null) {
  return count === null ? "Unavailable" : count.toLocaleString();
}

export function CleanupUsageSection() {
  const [usage, setUsage] = useState<CleanupUsage>();
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let active = true;
    let request = 0;
    let unsubscribe: (() => void) | undefined;
    const refresh = () => {
      const version = ++request;
      void ipc.getCleanupUsage().then(
        (summary) => {
          if (!active || request !== version) return;
          setUsage(summary);
          setFailed(false);
        },
        () => { if (active && request === version) setFailed(true); },
      );
    };
    void listen("cleanup-usage-updated", refresh).then((stop) => {
      if (!active) {
        stop?.();
        return;
      }
      unsubscribe = stop;
      refresh();
    }, () => { if (active) refresh(); });
    return () => {
      active = false;
      unsubscribe?.();
    };
  }, []);

  const latest = usage?.latest;
  return (
    <section className="prefs__section">
      <h2 className="prefs__heading">Token usage</h2>
      {failed ? <p>Usage unavailable</p> : !usage ? <p>Loading usage…</p> : (
        <>
          <div className="prefs__row">
            <span>Latest cleanup</span>
            {latest ? (
              <div>
                <div>{providerNames[latest.provider]} · {latest.model}</div>
                {latest.counts.inputTokens === null && latest.counts.outputTokens === null
                  ? "Usage unavailable"
                  : `Input ${format(latest.counts.inputTokens)} · Output ${format(latest.counts.outputTokens)}`}
              </div>
            ) : <span>No cleanup attempts yet</span>}
          </div>
          <div className="prefs__row">
            <span>Reported tokens on this Mac</span>
            <span>Input {format(usage.inputTokens)} · Output {format(usage.outputTokens)}</span>
          </div>
        </>
      )}
    </section>
  );
}

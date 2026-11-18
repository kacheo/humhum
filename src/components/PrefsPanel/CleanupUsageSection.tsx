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
          <table className="usage-table">
            <caption>Reported tokens on this Mac</caption>
            <thead>
              <tr><th scope="col">Cleanup</th><th scope="col">Input</th><th scope="col">Output</th></tr>
            </thead>
            <tbody>
              <tr>
                <th scope="row">Latest cleanup
                  {latest && <small>{providerNames[latest.provider]} · {latest.model}</small>}
                </th>
                {latest ? <>
                  <td>{format(latest.counts.inputTokens)}</td>
                  <td>{format(latest.counts.outputTokens)}</td>
                </> : <td colSpan={2}>No cleanup attempts yet</td>}
              </tr>
              <tr><th scope="row">All time</th><td>{format(usage.inputTokens)}</td><td>{format(usage.outputTokens)}</td></tr>
            </tbody>
          </table>
        </>
      )}
    </section>
  );
}

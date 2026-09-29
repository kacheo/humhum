import { useEffect, useState } from "react";
import { usePrefsStore } from "../stores/prefs";

/**
 * Hydrate the Zustand stores from the backend once on window mount by
 * delegating to their own IPC actions (the stores own their commands).
 * A preference load failure does not block the window.
 * Returns `ready` once the initial load settles, so a window can hold
 * rendering until stores reflect real data.
 */
export function useAppBootstrap(): { ready: boolean } {
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let active = true;
    (async () => {
      await usePrefsStore.getState().loadPrefs().catch(() => {});
      if (!active) return;
      setReady(true);
    })();
    return () => {
      active = false;
    };
  }, []);

  return { ready };
}

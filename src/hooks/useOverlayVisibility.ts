import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Drive the overlay window's visibility from the pipeline state. Shows without
 *  focus — the overlay is a passive HUD and must never steal focus from the app
 *  the user is dictating into. Keyed on `visible` so it only fires on change. */
export function useOverlayVisibility(visible: boolean): void {
  useEffect(() => {
    const w = getCurrentWindow();
    (visible ? w.show() : w.hide()).catch(() => {});
  }, [visible]);
}

import { useEffect, useState } from "react";
import { ipc } from "../../lib/ipc";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import type { PermissionsState } from "../../lib/types";
import { PermissionRecoveryHint } from "../PermissionRecoveryHint";

// Optimistic default so granted rows don't flash "Needed" before the first
// check resolves.
const ASSUME_GRANTED: PermissionsState = {
  microphone: true,
  accessibility: true,
  inputMonitoring: true,
};

type PermKey = keyof PermissionsState;

interface Row {
  key: PermKey;
  label: string;
  action: string;
  run: () => void;
}

/** The macOS permissions Wipzi needs, each with a one-click way to grant
 *  it when missing. This is the only in-app path to (re-)grant **Microphone**
 *  after onboarding — macOS won't let you add an app to the Microphone list
 *  manually, so it must be requested via the system prompt. Stays live via the
 *  `permissions-changed` event; the backend auto-registers the hotkey when
 *  the effective grants change. Some macOS grants need a full app restart. */
export function PermissionsNotice() {
  const [perms, setPerms] = useState<PermissionsState>(ASSUME_GRANTED);

  useEffect(() => {
    ipc.checkPermissions().then(setPerms).catch(() => {});
  }, []);
  useTauriEvent<PermissionsState>("permissions-changed", (e) => setPerms(e.payload));

  // Microphone is the only one that can be requested in-app (system prompt);
  // the other two deep-link to their System Settings pane.
  const rows: Row[] = [
    { key: "microphone", label: "Microphone", action: "Grant", run: () => { void ipc.requestMicrophone().catch(() => {}); } },
    { key: "accessibility", label: "Accessibility", action: "Open Settings", run: () => ipc.openAccessibilitySettings() },
    { key: "inputMonitoring", label: "Input Monitoring", action: "Open Settings", run: () => ipc.openInputMonitoringSettings() },
  ];
  const allGranted = rows.every((r) => perms[r.key]);

  return (
    <ul className="prefs__perm-list">
      {rows.map((r) => {
        const granted = perms[r.key];
        return (
          <li key={r.key} className={`prefs__perm-row${granted ? "" : " prefs__perm-row--missing"}`}>
            <span className="prefs__perm-label">{r.label}</span>
            <span className="prefs__perm-status" role="status">{granted ? "Granted ✓" : "Needed"}</span>
            {!granted && (
              <button type="button" onClick={r.run}>{r.action}</button>
            )}
          </li>
        );
      })}
      {!allGranted && (
        <li className="prefs__perm-hint">
          Microphone is needed to record; Accessibility and Input Monitoring run the global shortcut.
        </li>
      )}
      {(!perms.accessibility || !perms.inputMonitoring) && (
        <li className="prefs__perm-hint"><PermissionRecoveryHint perms={perms} /></li>
      )}
    </ul>
  );
}

import { useEffect, useState } from "react";
import { ipc } from "../../lib/ipc";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import type { PermissionsState } from "../../lib/types";
import { PermissionRecoveryHint } from "../PermissionRecoveryHint";

const ALL_FALSE: PermissionsState = { microphone: false, accessibility: false, inputMonitoring: false };

/** First onboarding step. Checks the three macOS permissions on mount and keeps
 *  them live via the `permissions-changed` event (Rust polls every 2s). Reports
 *  ready only when all three are granted. */
export function PermissionsStep({ onReady }: { onReady: (ready: boolean) => void }) {
  const [perms, setPerms] = useState<PermissionsState>(ALL_FALSE);

  useEffect(() => { ipc.checkPermissions().then(setPerms).catch(() => {}); }, []);
  useTauriEvent<PermissionsState>("permissions-changed", (e) => setPerms(e.payload));

  const allGranted = perms.microphone && perms.accessibility && perms.inputMonitoring;
  useEffect(() => { onReady(allGranted); }, [allGranted, onReady]);

  return (
    <section className="onb-step">
      <h2>Permissions</h2>
      <p>Humhum needs three macOS permissions to record audio and paste text.</p>
      <ul className="perm-list">
        <PermRow label="Microphone" granted={perms.microphone} action="Grant"
          onClick={() => { void ipc.requestMicrophone().catch(() => {}); }} />
        <PermRow label="Accessibility" granted={perms.accessibility} action="Open Settings"
          onClick={() => ipc.openAccessibilitySettings()} />
        <PermRow label="Input Monitoring" granted={perms.inputMonitoring} action="Open Settings"
          onClick={() => ipc.openInputMonitoringSettings()} />
      </ul>
      <PermissionRecoveryHint perms={perms} />
    </section>
  );
}

function PermRow({ label, granted, action, onClick }: {
  label: string; granted: boolean; action: string; onClick: () => void;
}) {
  return (
    <li className={`perm-row${granted ? " perm-row--granted" : ""}`}>
      <span className="perm-row__label">{label}</span>
      <span className="perm-row__status" role="status">{granted ? "Granted ✓" : "Needed"}</span>
      {!granted && <button type="button" onClick={onClick}>{action}</button>}
    </li>
  );
}

import { useEffect, useState } from "react";
import { ipc } from "../../lib/ipc";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import type { PermissionsState } from "../../lib/types";
import { Icon } from "../Icon";
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
      <h2 tabIndex={-1}>Permissions</h2>
      <p>Grant these macOS permissions so Humhum can record your voice, detect your shortcut, and paste the transcript.</p>
      <ul className="perm-list" role="list">
        <PermRow label="Microphone" granted={perms.microphone} action="Grant" primary
          onClick={() => { void ipc.requestMicrophone().catch(() => {}); }} />
        <PermRow label="Accessibility" granted={perms.accessibility} action="Open Settings" primary={perms.microphone}
          actionLabel="Open Settings for Accessibility"
          onClick={() => ipc.openAccessibilitySettings()} />
        <PermRow label="Input Monitoring" granted={perms.inputMonitoring} action="Open Settings" primary={perms.microphone && perms.accessibility}
          actionLabel="Open Settings for Input Monitoring"
          onClick={() => ipc.openInputMonitoringSettings()} />
      </ul>
      <PermissionRecoveryHint perms={perms} />
    </section>
  );
}

function PermRow({ label, granted, action, actionLabel, primary, onClick }: {
  label: string; granted: boolean; action: string; actionLabel?: string; primary: boolean; onClick: () => void;
}) {
  return (
    <li className={`perm-row${granted ? " perm-row--granted" : ""}`}>
      <span className="perm-row__label">{label}</span>
      <span className="perm-row__status" role="status">
        <span className="onboarding__sr-only">{label}: </span>
        <Icon name={granted ? "check" : "warning"} size={14} />
        {granted ? "Granted" : "Needed"}
      </span>
      {!granted && (
        <button type="button" className={primary ? "primary-action" : undefined} aria-label={actionLabel} onClick={onClick}>
          {action}
        </button>
      )}
    </li>
  );
}

import type { PermissionsState } from "../lib/types";

/** Recovery for a System Settings toggle that hasn't become an effective grant. */
export function PermissionRecoveryHint({ perms }: { perms: PermissionsState }) {
  if (perms.accessibility && perms.inputMonitoring) return null;

  return (
    <div role="note" aria-label="Permission recovery">
      <details>
        <summary>Permission still marked Needed?</summary>
        <p>Use the menu-bar menu to fully quit Humhum, then reopen it from Applications.</p>
        {!perms.accessibility && (
          <p>If Accessibility still shows Needed, remove Humhum from the system list and add the installed app again.</p>
        )}
      </details>
    </div>
  );
}

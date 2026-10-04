import type { PermissionsState } from "../lib/types";

/** Recovery for a System Settings toggle that hasn't become an effective grant. */
export function PermissionRecoveryHint({ perms }: { perms: PermissionsState }) {
  if (perms.accessibility && perms.inputMonitoring) return null;

  return (
    <div role="note" aria-label="Permission recovery">
      <p>
        If a permission is enabled in System Settings but still says Needed, fully quit Wipzi
        from its menu-bar menu and reopen the installed app in Applications.
        Closing this window does not quit the app.
      </p>
      {!perms.accessibility && (
        <p>
          If Accessibility still says Needed, remove Wipzi from the Accessibility list using
          the minus button. Then use Open Settings here and enable Wipzi again.
          Choose the installed app in Applications, rather than a development copy or the app inside the DMG.
        </p>
      )}
    </div>
  );
}

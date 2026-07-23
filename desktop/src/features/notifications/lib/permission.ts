import type { DesktopNotificationPermissionState } from "./desktop";

type EnsureDesktopNotificationPermissionOptions = {
  currentPermission: DesktopNotificationPermissionState;
  isWindows: boolean;
  requestAccess: () => Promise<DesktopNotificationPermissionState>;
};

/**
 * Requests access for the normal default state and retries Windows' known
 * false-denied notification shim state.
 */
export async function ensureDesktopNotificationPermission({
  currentPermission,
  isWindows,
  requestAccess,
}: EnsureDesktopNotificationPermissionOptions): Promise<DesktopNotificationPermissionState> {
  if (
    currentPermission === "default" ||
    (currentPermission === "denied" && isWindows)
  ) {
    return requestAccess();
  }

  return currentPermission;
}

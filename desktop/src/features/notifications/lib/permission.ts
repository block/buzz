import type { DesktopNotificationPermissionState } from "./desktop";

type EnsureDesktopNotificationPermissionOptions = {
  currentPermission: DesktopNotificationPermissionState;
  isWindowsTauri: boolean;
  requestAccess: () => Promise<DesktopNotificationPermissionState>;
};

/**
 * Requests access for the normal default state and retries the Windows Tauri
 * notification shim's known false-denied state.
 */
export async function ensureDesktopNotificationPermission({
  currentPermission,
  isWindowsTauri,
  requestAccess,
}: EnsureDesktopNotificationPermissionOptions): Promise<DesktopNotificationPermissionState> {
  if (
    currentPermission === "default" ||
    (currentPermission === "denied" && isWindowsTauri)
  ) {
    return requestAccess();
  }

  return currentPermission;
}

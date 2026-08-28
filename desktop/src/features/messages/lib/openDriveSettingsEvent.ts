/**
 * Cross-cutting "open Settings → Voice so the user can connect Google Drive"
 * signal, dispatched from the media-upload fallback toast when the relay's
 * media store is unavailable and the user has no Drive connected. AppShell owns
 * the settings route, so it subscribes and calls `goSettings("voice")`.
 *
 * Same window-CustomEvent shape as `openCreateAgentEvent.ts`, minus the
 * pending-value buffering: the AppShell listener is always mounted, so there is
 * no "dispatched before anyone was listening" case to replay.
 */
const OPEN_DRIVE_SETTINGS_EVENT = "buzz:open-drive-settings";

export function requestConnectGoogleDrive() {
  window.dispatchEvent(new Event(OPEN_DRIVE_SETTINGS_EVENT));
}

export function subscribeConnectGoogleDrive(handler: () => void) {
  window.addEventListener(OPEN_DRIVE_SETTINGS_EVENT, handler);
  return () => {
    window.removeEventListener(OPEN_DRIVE_SETTINGS_EVENT, handler);
  };
}

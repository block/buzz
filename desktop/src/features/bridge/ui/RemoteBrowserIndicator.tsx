// Desktop-only banner: shown whenever a browser session is visible to the member's web/mobile app (slice R).
import * as React from "react";

import { remoteBrowserManager, stopAllRemoteBrowserSessions } from "../browserHost";

export function RemoteBrowserIndicator() {
  const sessions = React.useSyncExternalStore(
    (cb) => remoteBrowserManager.subscribe(cb),
    () => remoteBrowserManager.list().length,
  );
  if (sessions === 0) return null;
  return (
    <div
      role="status"
      data-testid="remote-browser-indicator"
      className="pointer-events-auto fixed bottom-3 left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-full border bg-background px-4 py-2 text-sm shadow-lg"
    >
      <span className="h-2 w-2 animate-pulse rounded-full bg-red-500" />
      <span>
        {sessions === 1 ? "1 browser session is" : `${sessions} browser sessions are`} remotely
        visible — your phone or web app can request screenshots.
      </span>
      <button
        type="button"
        className="rounded-md border px-2 py-0.5 hover:bg-muted"
        onClick={() => void stopAllRemoteBrowserSessions()}
      >
        Stop sharing
      </button>
    </div>
  );
}

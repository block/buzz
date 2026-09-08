import * as React from "react";

import { getCachedMediaProxyPort, subscribeMediaProxyPort } from "./mediaUrl";

/**
 * The resolved localhost media-proxy port, re-rendering when it resolves or
 * changes. Components that call `rewriteRelayUrl` during render can subscribe
 * to this so an initial `buzz-media://` fallback is replaced by the loopback
 * proxy URL as soon as the Tauri backend reports the port.
 */
export function useMediaProxyPort(): number | null {
  return React.useSyncExternalStore(
    subscribeMediaProxyPort,
    getCachedMediaProxyPort,
    () => null,
  );
}

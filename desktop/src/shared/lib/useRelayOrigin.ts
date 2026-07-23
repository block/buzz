import * as React from "react";

import { getCachedRelayOrigin, subscribeRelayOrigin } from "./mediaUrl";

/**
 * The resolved relay origin, re-rendering when it resolves or changes.
 *
 * The origin is fetched asynchronously (see `mediaUrl.ts`) and is commonly
 * still `null` on a component's first render. Reading it through this store
 * subscription — rather than a bare synchronous `getCachedRelayOrigin()` —
 * means download eligibility recomputes the moment the origin resolves and
 * again on a workspace switch, instead of being frozen at first-render time.
 *
 * The server snapshot is `null`: there is no relay origin during SSR/prerender
 * (no Tauri backend), and callers already treat an unresolved origin as
 * "not yet downloadable" (fail closed).
 */
export function useRelayOrigin(): string | null {
  return React.useSyncExternalStore(
    subscribeRelayOrigin,
    getCachedRelayOrigin,
    () => null,
  );
}

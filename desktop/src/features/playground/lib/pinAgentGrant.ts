import type { BrowserAgentGrant } from "@/features/browser-agent/lib/types";

/** Match a playground pin sid to an active Observe/Drive grant. */
export function findGrantForPlaygroundPin(
  grants: readonly BrowserAgentGrant[],
  sid: string,
): BrowserAgentGrant | null {
  for (const grant of grants) {
    if (grant.surfaceId === sid) return grant;
    if (grant.webviewLabel === `playground-${sid}`) return grant;
  }
  return null;
}

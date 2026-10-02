import type { Community } from "./types";

/** Stable dependency for the backend's user-configured avatar source origins. */
export function communityRelaySetKey(
  communities: readonly Pick<Community, "relayUrl">[],
): string {
  const origins = communities.flatMap(({ relayUrl }) => {
    try {
      const url = new URL(relayUrl.replace(/^ws(s?):/, "http$1:"));
      if (
        !["http:", "https:"].includes(url.protocol) ||
        url.username ||
        url.password ||
        url.pathname !== "/" ||
        url.search ||
        url.hash
      ) {
        return [];
      }
      return [url.origin];
    } catch {
      return [];
    }
  });
  return JSON.stringify([...new Set(origins)].sort());
}

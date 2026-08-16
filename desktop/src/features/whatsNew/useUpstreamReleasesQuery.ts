import { useQuery } from "@tanstack/react-query";

import { parseUpstreamReleases } from "@/features/whatsNew/lib/upstreamReleases.mjs";
import type { ReleaseTimelineRow } from "@/features/whatsNew/lib/upstreamReleases.mjs";

const UPSTREAM_RELEASES_URL =
  "https://api.github.com/repos/block/buzz/releases?per_page=100";

/**
 * Upstream Buzz's published desktop releases, capped at the version this fork
 * has caught up to.
 *
 * Deliberately quiet about failure. This is supplementary history on a
 * settings page, not something the app depends on, and the two most likely
 * failures — offline, or GitHub's 60-requests-per-hour unauthenticated limit —
 * are both routine. On either the caller simply renders the fork's own
 * history, so no retry storm and no error surface.
 *
 * The long `staleTime` is what keeps a user who opens Settings repeatedly from
 * burning that hourly allowance; release history changes at most weekly, so
 * refetching more often buys nothing.
 */
export function useUpstreamReleasesQuery(coreVersion: string | null) {
  return useQuery<ReleaseTimelineRow[]>({
    queryKey: ["upstream-releases", coreVersion],
    queryFn: async () => {
      const response = await fetch(UPSTREAM_RELEASES_URL, {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (!response.ok) {
        // Includes 403 for rate limiting. Nothing here is worth escalating.
        throw new Error(`GitHub responded ${response.status}`);
      }
      return parseUpstreamReleases(await response.json(), coreVersion);
    },
    // Without the app's own version there is no ceiling, and listing every
    // upstream release would advertise features this build does not have.
    enabled: coreVersion !== null,
    staleTime: 6 * 60 * 60 * 1_000,
    gcTime: 24 * 60 * 60 * 1_000,
    retry: false,
    refetchOnWindowFocus: false,
  });
}

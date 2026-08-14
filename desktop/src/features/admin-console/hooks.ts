import { useQuery } from "@tanstack/react-query";

import { useIdentityQuery } from "@/shared/api/hooks";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";
import { discoverAdminOrigin, getAdminOrigin, probeAdminOrigin } from "./api";
import type { ModerationNavResolution } from "./nav";

export const moderationNavResolutionQueryKey = (
  pubkeyHex: string,
  relayOrigin: string | null,
) => ["moderationNavResolution", pubkeyHex, relayOrigin] as const;

/**
 * Resolve the origin + probe state that decide whether the Moderation nav
 * entry is visible. Mirrors the settings card's mount resolution: a saved
 * manual origin wins outright (and short-circuits the probe, since the gate
 * shows the entry regardless); otherwise NIP-11 discovery is attempted and,
 * when it advertises an origin, probed so the gate can distinguish an
 * authorized relay from a definitive non-admin verdict.
 *
 * Keyed by pubkey **and the connected relay origin**: NIP-11 discovery is
 * relay-dependent, so a pubkey-only key would serve the previous relay's
 * verdict for up to `staleTime` after a workspace switch. Gating `enabled`
 * on the relay origin also defers resolution until the relay identity is
 * known, so no verdict is computed against an unresolved relay. Errors from
 * the probe resolve to a `"error"` outcome rather than rejecting — a
 * transport flake must keep the entry visible, not make the whole query fail.
 */
export function useModerationNavResolution():
  | ModerationNavResolution
  | undefined {
  const { data: identity } = useIdentityQuery();
  const pubkeyHex = identity?.pubkey ?? "";
  const relayOrigin = useRelayOrigin();

  const query = useQuery({
    enabled: pubkeyHex.length > 0 && relayOrigin != null,
    queryKey: moderationNavResolutionQueryKey(pubkeyHex, relayOrigin),
    staleTime: 60_000,
    queryFn: async (): Promise<ModerationNavResolution> => {
      const saved = await getAdminOrigin(pubkeyHex);
      if (saved) {
        return { originSource: "saved", probe: null };
      }
      let discovered: string | null = null;
      try {
        discovered = await discoverAdminOrigin();
      } catch {
        discovered = null;
      }
      if (!discovered) {
        return { originSource: "none", probe: null };
      }
      try {
        const result = await probeAdminOrigin(discovered);
        return { originSource: "advertised", probe: result.state };
      } catch {
        return { originSource: "advertised", probe: "error" };
      }
    },
  });

  return query.data;
}

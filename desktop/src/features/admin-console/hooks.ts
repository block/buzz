import { useQuery } from "@tanstack/react-query";

import { useIdentityQuery } from "@/shared/api/hooks";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";
import { discoverAdminOrigin, getAdminOrigin } from "./api";
import type { RelayAdminNavResolution } from "./nav";

export const moderationNavResolutionQueryKey = (
  pubkeyHex: string,
  relayOrigin: string | null,
) => ["moderationNavResolution", pubkeyHex, relayOrigin] as const;

/**
 * Resolve the origin source that decides whether the Admin nav entry is
 * visible. A saved manual origin wins outright; otherwise NIP-11 discovery is
 * attempted and, when it advertises an origin, the entry is shown so the
 * operator can open Admin and see the panel (the card will auto-save and
 * auto-probe the discovered origin on first open).
 *
 * Keyed by pubkey **and the connected relay origin**: NIP-11 discovery is
 * relay-dependent, so a pubkey-only key would serve the previous relay's
 * verdict for up to `staleTime` after a workspace switch. Gating `enabled`
 * on the relay origin also defers resolution until the relay identity is
 * known, so no verdict is computed against an unresolved relay.
 */
export function useModerationNavResolution():
  | RelayAdminNavResolution
  | undefined {
  const { data: identity } = useIdentityQuery();
  const pubkeyHex = identity?.pubkey ?? "";
  const relayOrigin = useRelayOrigin();

  const query = useQuery({
    enabled: pubkeyHex.length > 0 && relayOrigin != null,
    queryKey: moderationNavResolutionQueryKey(pubkeyHex, relayOrigin),
    staleTime: 60_000,
    queryFn: async (): Promise<RelayAdminNavResolution> => {
      const saved = await getAdminOrigin(pubkeyHex);
      if (saved) {
        return { originSource: "saved" };
      }
      let discovered: string | null = null;
      try {
        discovered = await discoverAdminOrigin();
      } catch {
        discovered = null;
      }
      return { originSource: discovered ? "advertised" : "none" };
    },
  });

  return query.data;
}

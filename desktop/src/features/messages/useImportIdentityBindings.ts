import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import { relayClient } from "@/shared/api/relayClient";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_IMPORT_IDENTITY_BINDING } from "@/shared/constants/kinds";

/**
 * Owner/admin-signed import identity bindings for the active community:
 * `<source>:<foreign id>` (e.g. `slack:U060`) → the bound Buzz pubkey
 * (lowercase hex). Feeds `formatTimelineMessages` so bot-signed imported
 * history renders under the real person's profile.
 *
 * The relay only stores this kind when signed by an owner/admin, so every
 * binding served here is already authoritative — no client-side trust check
 * is needed beyond taking the newest binding per key.
 */
const importIdentityBindingsQueryKey = ["import-identity-bindings"] as const;

function buildBindingMap(events: RelayEvent[]): Map<string, string> {
  // Newest binding per key wins: sort ascending so later writes overwrite.
  const ordered = [...events].sort((a, b) => a.created_at - b.created_at);
  const map = new Map<string, string>();
  for (const event of ordered) {
    const dTag = event.tags.find((t) => t[0] === "d")?.[1];
    const pubkey = event.tags.find((t) => t[0] === "p")?.[1];
    if (!dTag || !pubkey) continue;
    if (pubkey.length !== 64) continue;
    map.set(dTag, pubkey.toLowerCase());
  }
  return map;
}

const EMPTY_PUBKEYS: string[] = [];

/**
 * Returns the binding map plus the deduped list of bound pubkeys — the latter
 * so callers can add those people to their profile batch fetch and render
 * imported history under the right avatar. Both are stable across renders.
 */
export function useImportIdentityBindings(): {
  bindings: Map<string, string> | undefined;
  boundPubkeys: string[];
} {
  const query = useQuery({
    queryKey: importIdentityBindingsQueryKey,
    queryFn: async () => {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_IMPORT_IDENTITY_BINDING],
        limit: 1000,
      });
      return buildBindingMap(events);
    },
    // Bindings change rarely (only when an operator attributes an import).
    staleTime: 5 * 60_000,
  });
  const bindings = query.data;
  const boundPubkeys = useMemo(
    () => (bindings ? [...bindings.values()] : EMPTY_PUBKEYS),
    [bindings],
  );
  return { bindings, boundPubkeys };
}

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_IMPORT_IDENTITY_BINDING,
  KIND_IMPORT_IDENTITY_CLAIM,
} from "@/shared/constants/kinds";
import { buildConfirmedImportBindings } from "./lib/confirmImportBindings";

/**
 * Confirmed import identity bindings for the active community:
 * `<source>:<foreign id>` (e.g. `slack:U060`) → the bound Buzz pubkey
 * (lowercase hex). Feeds `formatTimelineMessages` so bot-signed imported
 * history renders under the real person's profile.
 *
 * Attribution is two-party (see {@link buildConfirmedImportBindings}): a key is
 * confirmed only when an owner/admin attestation and the subject's own claim
 * agree. So a member can't attest another person's history, and an admin can't
 * make someone appear to author history they never wrote.
 */
export const importIdentityBindingsQueryKey = [
  "import-identity-bindings",
] as const;

const EMPTY_PUBKEYS: string[] = [];

/**
 * Returns the confirmed binding map plus the deduped list of bound pubkeys —
 * the latter so callers can add those people to their profile batch fetch and
 * render imported history under the right avatar. Both are stable across
 * renders.
 */
export function useImportIdentityBindings(): {
  bindings: Map<string, string> | undefined;
  boundPubkeys: string[];
} {
  const query = useQuery({
    queryKey: importIdentityBindingsQueryKey,
    queryFn: async () => {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_IMPORT_IDENTITY_BINDING, KIND_IMPORT_IDENTITY_CLAIM],
        limit: 2000,
      });
      return buildConfirmedImportBindings(events);
    },
    // Bindings change rarely (only when an operator attributes an import or a
    // person consents).
    staleTime: 5 * 60_000,
  });
  const bindings = query.data;
  const boundPubkeys = useMemo(
    () => (bindings ? [...bindings.values()] : EMPTY_PUBKEYS),
    [bindings],
  );
  return { bindings, boundPubkeys };
}

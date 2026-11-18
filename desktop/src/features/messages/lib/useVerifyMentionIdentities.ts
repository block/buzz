import * as React from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";

import {
  USERS_BATCH_ENTRY_FRESH_MS,
  usersBatchEntryKey,
  type UsersBatchEntry,
} from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { getUsersBatch } from "@/shared/api/tauriProfiles";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { collectProfileAliases } from "@/shared/lib/resolveMentionNames";

import type { MentionCandidate } from "./mentionCandidates";
import type {
  MentionIdentity,
  VerifyMentionIdentities,
} from "./mentionClipboard";
import { partitionMentionIdentitiesByLocalTrust } from "./mentionIdentityTrust";

/**
 * Ask the relay which names it holds for each pubkey.
 *
 * Reads the per-pubkey entries `useUsersBatchQuery` maintains before spending
 * a request: the surface a mention was copied *from* already resolved that
 * profile to render its chip, so the common cross-channel paste answers from
 * cache. Entries this side considers stale are refetched rather than trusted.
 *
 * Read-only on the cache. Seeding it from here would hand the profile hooks a
 * resolved entry they never wrote the offline label for.
 *
 * The caller only ever passes records that survived
 * `parseMentionClipboardRecords`, so `pubkeys` is bounded by its record cap —
 * a hostile clipboard cannot turn one paste into an unbounded fan-out.
 */
async function fetchTrustedMentionAliases(
  queryClient: QueryClient,
  pubkeys: readonly string[],
): Promise<Map<string, string[]>> {
  const aliases = new Map<string, string[]>();
  const toFetch: string[] = [];
  const now = Date.now();
  for (const pubkey of new Set(pubkeys)) {
    const entry = queryClient.getQueryData<UsersBatchEntry>(
      usersBatchEntryKey(pubkey),
    );
    if (entry && now - entry.fetchedAt < USERS_BATCH_ENTRY_FRESH_MS) {
      aliases.set(pubkey, collectProfileAliases(entry.summary ?? undefined));
    } else {
      toFetch.push(pubkey);
    }
  }
  if (toFetch.length === 0) return aliases;

  const fresh = await getUsersBatch(toFetch);
  for (const pubkey of toFetch) {
    aliases.set(pubkey, collectProfileAliases(fresh.profiles[pubkey]));
  }
  return aliases;
}

/**
 * Build the paste-side check that a copied `label → pubkey` pair is one this
 * community actually holds — see `mentionIdentityTrust` for why a pair the
 * clipboard merely *shows* is not evidence of anything.
 *
 * Two sources, in cost order:
 *
 * 1. Local trusted state — the mention candidates the composer would offer
 *    (channel roster, agent and persona directories, relay user search) and
 *    the profile lookup the surrounding surface renders from. Both are relay
 *    output, and both are already in hand.
 * 2. The relay's own profile for the pubkey, for a pair no local directory can
 *    speak to. This is the case the feature exists for: a mention of someone
 *    who is not a member of the channel being pasted into.
 *
 * Anything neither source names is dropped. A pair naming a key this community
 * has never seen — the shape a forged clipboard record takes — stays plain
 * text.
 */
export function useVerifyMentionIdentities({
  mentionCandidates,
  profiles,
}: {
  mentionCandidates: readonly MentionCandidate[];
  profiles: UserProfileLookup | undefined;
}): VerifyMentionIdentities {
  const queryClient = useQueryClient();

  const resolveLocalAliases = React.useCallback(
    (pubkey: string): string[] => {
      const aliases = collectProfileAliases(profiles?.[pubkey]);
      for (const candidate of mentionCandidates) {
        if (!candidate.pubkey) continue;
        if (normalizePubkey(candidate.pubkey) !== pubkey) continue;
        // A managed agent renders under its persona name, which is no alias of
        // its kind-0 profile — so a copy of that chip has to be verifiable
        // against the directory that named it.
        if (candidate.displayName) aliases.push(candidate.displayName);
        if (candidate.personaName) aliases.push(candidate.personaName);
      }
      return aliases;
    },
    [mentionCandidates, profiles],
  );

  return React.useCallback(
    async (records: readonly MentionIdentity[]) => {
      const normalized = records.map((record) => ({
        ...record,
        pubkey: normalizePubkey(record.pubkey),
      }));
      const { trusted, unresolved } = partitionMentionIdentitiesByLocalTrust(
        normalized,
        resolveLocalAliases,
      );
      if (unresolved.length === 0) return trusted;

      const relayAliases = await fetchTrustedMentionAliases(
        queryClient,
        unresolved.map((record) => record.pubkey),
      );
      const { trusted: vouched } = partitionMentionIdentitiesByLocalTrust(
        unresolved,
        (pubkey) => relayAliases.get(pubkey) ?? [],
      );
      return [...trusted, ...vouched];
    },
    [queryClient, resolveLocalAliases],
  );
}

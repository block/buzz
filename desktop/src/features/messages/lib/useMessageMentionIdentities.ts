import * as React from "react";

import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { resolveMentionProps } from "@/shared/lib/resolveMentionNames";

import type { MentionIdentity } from "./mentionClipboard";

/**
 * The `label → pubkey` pairs a delivered message tagged.
 *
 * Same alias set the renderer chips against (`resolveMentionProps`), so any
 * `@name` the body shows resolves to the identity the author actually tagged
 * — which is exactly what a copy needs to carry.
 */
export function useMessageMentionIdentities(
  tags: string[][] | undefined,
  profiles: UserProfileLookup | undefined,
): MentionIdentity[] {
  const knownAgentPubkeys = useKnownAgentPubkeys();
  return React.useMemo(() => {
    const { mentionNames, mentionPubkeysByName } = resolveMentionProps(
      tags,
      profiles,
    );
    if (!mentionNames || !mentionPubkeysByName) return [];
    const identities: MentionIdentity[] = [];
    for (const label of mentionNames) {
      const pubkey = mentionPubkeysByName[label.toLowerCase()];
      if (!pubkey) continue;
      const normalized = normalizePubkey(pubkey);
      identities.push({
        label,
        pubkey: normalized,
        isAgent:
          knownAgentPubkeys.has(normalized) ||
          profiles?.[normalized]?.isAgent === true,
      });
    }
    return identities;
  }, [knownAgentPubkeys, profiles, tags]);
}

/**
 * NIP-MR: inline mention-ack footers, extracted from ChannelPane.tsx so that
 * file stays inside the desktop file-size ratchet. The id list is
 * reference-stable and almost always empty, so this builds nothing on the
 * overwhelmingly common healthy path.
 */

import * as React from "react";

import { useProblemEventIds } from "@/features/agents/pendingMentionAckStore";
import { MentionAckFooter } from "@/features/agents/ui/MentionAckFooter";
import { truncatePubkey } from "@/shared/lib/pubkey";
import type { UserProfileLookup } from "@/features/profile/lib/identity";

/**
 * Human-readable name for a mentioned agent's owner: profile displayName,
 * then kind-0 `name`, then a short hex. Pure so it is unit-testable.
 */
export function resolveMentionAckDisplayName(
  profiles: UserProfileLookup | undefined,
  pubkey: string,
): string {
  return (
    profiles?.[pubkey]?.displayName ||
    profiles?.[pubkey]?.name ||
    truncatePubkey(pubkey)
  );
}

/**
 * Footer slots for mentions no agent picked up, keyed by event id for
 * `MessageTimeline`'s `messageFooters` prop. `undefined` on the healthy path.
 */
export function useChannelMentionAckFooters(
  channelId: string,
  profiles?: UserProfileLookup,
): Record<string, React.ReactNode> | undefined {
  const mentionAckProblemIds = useProblemEventIds(channelId);
  const resolveMentionAckName = React.useCallback(
    (pubkey: string) => resolveMentionAckDisplayName(profiles, pubkey),
    [profiles],
  );
  return React.useMemo(() => {
    if (mentionAckProblemIds.length === 0) return undefined;
    const footers: Record<string, React.ReactNode> = {};
    for (const eventId of mentionAckProblemIds) {
      footers[eventId] = React.createElement(MentionAckFooter, {
        eventId,
        resolveName: resolveMentionAckName,
      });
    }
    return footers;
  }, [mentionAckProblemIds, resolveMentionAckName]);
}

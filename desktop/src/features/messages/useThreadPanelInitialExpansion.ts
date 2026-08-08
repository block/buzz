import * as React from "react";

import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import { computeInitialExpandedReplyIds } from "@/features/messages/lib/threadPanel";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  Channel,
  ChannelMember,
  RelayEvent,
  RespondToMode,
} from "@/shared/api/types";

type UseThreadPanelInitialExpansionOptions = {
  activeChannel: Channel | null;
  threadReplyEvents: RelayEvent[];
  openThreadHeadId: string | null;
  setExpandedThreadReplyIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  currentPubkey: string | undefined;
  currentAvatarUrl: string | null;
  profiles: UserProfileLookup | undefined;
  members: ChannelMember[] | undefined;
  personaLookup: Map<string, string>;
  respondToLookup: Map<string, RespondToMode>;
  relaySelfPubkey: string | null | undefined;
  ownerProfiles: UserProfileLookup | undefined;
};

/**
 * Seeds the thread panel's initial ancestor expansion (block/buzz#3799).
 *
 * A thread panel opened on a thread whose subtree already contains depth-2+
 * replies (the classic "agent replied to a *non-latest* thread message"
 * case) would otherwise start with an empty expansion set and swallow those
 * replies inside a collapsed depth-1 summary row — invisible in the UI even
 * though the relay delivers them. This effect unions the ancestors of every
 * depth-2+ reply present into the expansion set, once per opened head, so
 * they render on the first paint. Manual expand/collapse afterwards still
 * wins because the union never removes ids. Kept out of ChannelScreen.tsx
 * to respect the desktop file-size ratchet.
 */
export function useThreadPanelInitialExpansion({
  activeChannel,
  threadReplyEvents,
  openThreadHeadId,
  setExpandedThreadReplyIds,
  currentPubkey,
  currentAvatarUrl,
  profiles,
  members,
  personaLookup,
  respondToLookup,
  relaySelfPubkey,
  ownerProfiles,
}: UseThreadPanelInitialExpansionOptions) {
  const seededHeadRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (!openThreadHeadId) {
      seededHeadRef.current = null;
      return;
    }
    if (seededHeadRef.current === openThreadHeadId) {
      return;
    }
    if (threadReplyEvents.length === 0) {
      return;
    }
    const formatted = formatTimelineMessages(
      threadReplyEvents,
      activeChannel,
      currentPubkey,
      currentAvatarUrl,
      profiles,
      members,
      personaLookup,
      respondToLookup,
      relaySelfPubkey,
      ownerProfiles,
    );
    const seed = computeInitialExpandedReplyIds(formatted, openThreadHeadId);
    if (seed.size === 0) {
      seededHeadRef.current = openThreadHeadId;
      return;
    }
    setExpandedThreadReplyIds((current) => {
      // Idempotent: if every seed id is already expanded (e.g. the relay
      // delivered the replies between open and fetch resolution), bail with
      // the identity reference so React skips the re-render.
      let allPresent = true;
      for (const id of seed) {
        if (!current.has(id)) {
          allPresent = false;
          break;
        }
      }
      if (allPresent) {
        return current;
      }
      const next = new Set(current);
      for (const id of seed) {
        next.add(id);
      }
      return next;
    });
    seededHeadRef.current = openThreadHeadId;
  }, [
    activeChannel,
    currentAvatarUrl,
    currentPubkey,
    members,
    openThreadHeadId,
    ownerProfiles,
    personaLookup,
    profiles,
    relaySelfPubkey,
    respondToLookup,
    setExpandedThreadReplyIds,
    threadReplyEvents,
  ]);
}

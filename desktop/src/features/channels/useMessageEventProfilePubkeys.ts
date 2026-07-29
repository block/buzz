import * as React from "react";

import {
  collectForwardEmbeddedPubkeys,
  collectMessageAuthorPubkeys,
  collectMessageMentionPubkeys,
  collectReactionActorPubkeys,
} from "@/features/messages/lib/formatTimelineMessages";
import type { RelayEvent } from "@/shared/api/types";

export function useMessageEventProfilePubkeys(
  messages: RelayEvent[],
  threadReplies: RelayEvent[],
  relaySelfPubkey: string | null | undefined,
) {
  return React.useMemo(() => {
    const events = [...messages, ...threadReplies];
    return [
      ...new Set([
        ...collectMessageAuthorPubkeys(events, relaySelfPubkey),
        ...collectMessageMentionPubkeys(events),
        // Forwarded rows render an embedded original: its author and mentions
        // ride this batch instead of a per-row query.
        ...collectForwardEmbeddedPubkeys(events),
        ...collectReactionActorPubkeys(events, relaySelfPubkey),
      ]),
    ];
  }, [messages, relaySelfPubkey, threadReplies]);
}

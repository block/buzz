import * as React from "react";

import {
  decideRequestMarking,
  type RequestMarking,
} from "@/features/messages/lib/requestMarking";

/**
 * Tracks whether the current draft will create a NIP-AD obligation, so the
 * composer can disclose it before the message goes out.
 *
 * The decision itself lives in `decideRequestMarking`, shared with the send
 * path. They were separate before — the send path derived a target from the
 * mention list and nothing in the UI said so — which is how "@agent thanks"
 * silently became an unanswered obligation, and how a two-agent request
 * silently became untracked chat.
 */
export function useComposerRequestMarking(
  isAgentPubkey: (pubkey: string) => boolean,
  getDisplayName: (pubkey: string) => string | null,
) {
  const [optedOut, setOptedOut] = React.useState(false);
  const [mentionedAgentPubkeys, setMentionedAgentPubkeys] = React.useState<
    string[]
  >([]);

  // A draft with no agent mention opts back in: the choice was about the
  // message that had one.
  React.useEffect(() => {
    if (mentionedAgentPubkeys.length === 0 && optedOut) {
      setOptedOut(false);
    }
  }, [mentionedAgentPubkeys.length, optedOut]);

  const syncFromText = React.useCallback(
    (pubkeys: string[]) => {
      setMentionedAgentPubkeys(pubkeys.filter(isAgentPubkey));
    },
    [isAgentPubkey],
  );

  const marking: RequestMarking = decideRequestMarking(
    mentionedAgentPubkeys,
    optedOut,
  );

  const disableRequest = React.useCallback(() => setOptedOut(true), []);

  return {
    marking,
    optedOut,
    syncFromText,
    /** Spread straight into `ComposerRequestBanner`. */
    bannerProps: {
      multiAgent: marking.kind === "multi-agent",
      targetLabel:
        marking.kind === "tracked"
          ? getDisplayName(marking.targetPubkey)
          : null,
      onDisableRequest: disableRequest,
    },
  };
}

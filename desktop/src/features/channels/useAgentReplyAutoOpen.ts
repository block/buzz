import * as React from "react";

import { decideAgentReplyAutoOpen } from "@/features/messages/lib/agentReplyAutoOpen";
import { useChannelSubscription } from "@/features/messages/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { Channel, RelayEvent } from "@/shared/api/types";

/**
 * Auto-open the originating thread when an agent replies to the user's most
 * recent top-level message that targeted an agent.
 *
 * Triggered exclusively by `onTopLevelMessageSent` (the screen wires it into
 * the send-callback chain). The hook stores the published event id when its
 * `p` tags target a known agent in the channel; the live subscription callback
 * consumes that one-shot id when a matching hidden reply arrives. Channel
 * navigation clears the trigger so hydration noise is not interpreted as a
 * "new" arrival.
 *
 * The hook owns the channel live subscription: agent-reply detection is the
 * sole consumer of the per-event callback, so the subscription lives with its
 * only reader rather than threading a feature-specific hook through the screen.
 */
export function useAgentReplyAutoOpen({
  activeChannel,
  agentPubkeys,
  hasActiveAuxiliaryPanel,
  relaySelfPubkey,
  setExpandedThreadReplyIds,
  setOpenThreadHeadId,
  setOptimisticOpenThreadHeadId,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
}: {
  /** Active channel — drives the live subscription and its id. */
  activeChannel: Channel | null;
  /** Normalized pubkeys of every known agent in the active channel. */
  agentPubkeys: ReadonlySet<string>;
  hasActiveAuxiliaryPanel: boolean;
  relaySelfPubkey?: string | null;
  setExpandedThreadReplyIds: React.Dispatch<React.SetStateAction<Set<string>>>;
  setOpenThreadHeadId: (value: string | null) => void;
  setOptimisticOpenThreadHeadId: React.Dispatch<
    React.SetStateAction<string | null | undefined>
  >;
  setThreadReplyTargetId: React.Dispatch<React.SetStateAction<string | null>>;
  setThreadScrollTargetId: React.Dispatch<React.SetStateAction<string | null>>;
}) {
  const recentAgentTargetMessageRef = React.useRef<string | null>(null);
  const activeChannelId = activeChannel?.id ?? null;
  const autoOpenChannelIdRef = React.useRef(activeChannelId);

  if (autoOpenChannelIdRef.current !== activeChannelId) {
    autoOpenChannelIdRef.current = activeChannelId;
    recentAgentTargetMessageRef.current = null;
  }

  const handleTopLevelMessageSent = React.useCallback(
    (event: RelayEvent | null) => {
      if (event === null) {
        return;
      }
      const pTags = (event.tags ?? []).filter((tag) => tag[0] === "p");
      const targetsAgent = pTags.some((tag) => {
        const pubkey = tag[1];
        if (typeof pubkey !== "string") {
          return false;
        }
        return agentPubkeys.has(normalizePubkey(pubkey));
      });
      if (!targetsAgent) {
        return;
      }
      recentAgentTargetMessageRef.current = event.id;
    },
    [agentPubkeys],
  );

  const handleLiveEvent = React.useCallback(
    (event: RelayEvent) => {
      const decision = decideAgentReplyAutoOpen({
        event,
        hasActiveAuxiliaryPanel,
        expectedRootId: recentAgentTargetMessageRef.current,
        agentPubkeys,
        relaySelfPubkey,
      });
      if (!decision.rootId || !decision.replyId) {
        return;
      }

      recentAgentTargetMessageRef.current = null;
      const rootId = decision.rootId;
      const replyId = decision.replyId;
      // Capture the channel the reply arrived on. The policy already checked
      // `hasActiveAuxiliaryPanel` at the moment the live event fired, but the
      // deferred setters can otherwise outlive a channel navigation and open a
      // thread in the new channel — bail when the active channel has changed.
      const scheduledChannelId = activeChannelId;
      window.setTimeout(() => {
        if (autoOpenChannelIdRef.current !== scheduledChannelId) {
          return;
        }
        React.startTransition(() => {
          setOptimisticOpenThreadHeadId(rootId);
          setOpenThreadHeadId(rootId);
          setThreadReplyTargetId(rootId);
          setThreadScrollTargetId(replyId);
          setExpandedThreadReplyIds(new Set());
        });
      }, 0);
    },
    [
      activeChannelId,
      agentPubkeys,
      hasActiveAuxiliaryPanel,
      relaySelfPubkey,
      setExpandedThreadReplyIds,
      setOpenThreadHeadId,
      setOptimisticOpenThreadHeadId,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    ],
  );

  useChannelSubscription(activeChannel, handleLiveEvent);

  return { handleTopLevelMessageSent };
}

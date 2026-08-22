import * as React from "react";

import { decideAgentReplyAutoOpen } from "@/features/messages/lib/agentReplyAutoOpen";
import { getChannelIdFromTags } from "@/features/messages/lib/threading";
import { useChannelSubscription } from "@/features/messages/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { Channel, RelayEvent } from "@/shared/api/types";

/**
 * Auto-open the originating thread when an agent replies to the user's most
 * recent top-level message that targeted an agent.
 *
 * Triggered exclusively by `onTopLevelMessageSent` (the screen wires it into
 * the send-callback chain). The hook stores the published event id together
 * with the channel the message was actually delivered to, taken from the
 * event's own `h` tag rather than from whichever channel happens to be active
 * when the send promise resolves. A send can outlive its originating channel:
 * mentioning an agent in a DM creates an expanded DM
 * (`usePrepareDmSendChannel`) and the pane navigates there once the send
 * settles, so a trigger keyed on the active channel would be armed and then
 * immediately discarded. The live subscription callback consumes that one-shot
 * trigger when a matching hidden reply arrives on the same channel.
 *
 * A failed send clears any pending trigger: the user's newest attempt is the
 * one they are waiting on, so a late reply to an older message must not steal
 * the panel.
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
  const recentAgentTargetRef = React.useRef<{
    rootId: string;
    channelId: string;
  } | null>(null);
  const activeChannelId = activeChannel?.id ?? null;
  const autoOpenChannelIdRef = React.useRef(activeChannelId);
  autoOpenChannelIdRef.current = activeChannelId;
  // Render-mirrored so the deferred timer below sees panel state as of its
  // firing, not as of the live event — a panel opened in between must win.
  const hasActiveAuxiliaryPanelRef = React.useRef(hasActiveAuxiliaryPanel);
  hasActiveAuxiliaryPanelRef.current = hasActiveAuxiliaryPanel;
  // Deferred opens are cancelled on unmount: the timer closes over state
  // setters and a URL-backed navigation, both of which must not run once the
  // screen is gone.
  const pendingTimersRef = React.useRef<Set<number>>(new Set());
  React.useEffect(() => {
    const timers = pendingTimersRef.current;
    return () => {
      for (const timer of timers) {
        window.clearTimeout(timer);
      }
      timers.clear();
    };
  }, []);

  const handleTopLevelMessageSent = React.useCallback(
    (event: RelayEvent | null) => {
      // A failed send supersedes whatever was armed before it: the user is
      // waiting on this attempt, not on an older one.
      if (event === null) {
        recentAgentTargetRef.current = null;
        return;
      }
      const tags = event.tags ?? [];
      const pTags = tags.filter((tag) => tag[0] === "p");
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
      // Bind to the channel the relay actually delivered to, not the active
      // channel: an expanded DM is created mid-send and navigated to after it.
      const deliveredChannelId = getChannelIdFromTags(tags);
      if (!deliveredChannelId) {
        return;
      }
      recentAgentTargetRef.current = {
        rootId: event.id,
        channelId: deliveredChannelId,
      };
    },
    [agentPubkeys],
  );

  const handleLiveEvent = React.useCallback(
    (event: RelayEvent) => {
      const pending = recentAgentTargetRef.current;
      // The reply must land on the same channel the armed message was
      // delivered to. Without this a send that resolves after the user has
      // navigated away could open a thread rooted in a different channel.
      if (
        pending === null ||
        getChannelIdFromTags(event.tags ?? []) !== pending.channelId
      ) {
        return;
      }

      const decision = decideAgentReplyAutoOpen({
        event,
        hasActiveAuxiliaryPanel,
        expectedRootId: pending.rootId,
        agentPubkeys,
        relaySelfPubkey,
      });
      if (!decision.rootId || !decision.replyId) {
        return;
      }

      recentAgentTargetRef.current = null;
      const rootId = decision.rootId;
      const replyId = decision.replyId;
      // The policy already checked `hasActiveAuxiliaryPanel` at the moment the
      // live event fired, but the deferred setters can otherwise outlive a
      // channel navigation or a panel the user opened in the interim — bail
      // when either changed.
      const scheduledChannelId = pending.channelId;
      const timer = window.setTimeout(() => {
        pendingTimersRef.current.delete(timer);
        if (
          autoOpenChannelIdRef.current !== scheduledChannelId ||
          hasActiveAuxiliaryPanelRef.current
        ) {
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
      pendingTimersRef.current.add(timer);
    },
    [
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

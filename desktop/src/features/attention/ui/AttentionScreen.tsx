import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useHomeFeedQuery } from "@/features/home/hooks";
import { buildInboxItems } from "@/features/home/lib/inbox";
import { useSendMessageMutation } from "@/features/messages/hooks";
import {
  actionPublishes,
  type AttentionItem,
  attentionThreadRootId,
  HANDLED_ELSEWHERE_REPLY,
  projectAttention,
} from "@/features/attention/lib/attention";
import type { AttentionCardAction } from "@/features/attention/ui/AttentionCard";
import { AttentionView } from "@/features/attention/ui/AttentionView";
import { useActionQueue } from "@/features/attention/useActionQueue";
import { useAttentionZoneState } from "@/features/attention/useAttentionZoneState";
import { useBadgeOverrides } from "@/features/attention/useBadgeOverrides";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  isRelayUnreachableError,
  RELAY_UNREACHABLE_MESSAGE,
} from "@/shared/lib/relayError";

/** Canned prose for the park actions. Replies stay human-readable. */
const ACTION_CONFIG: Record<
  AttentionCardAction,
  { reply: string; toast: string; zone: "done" | "waiting" }
> = {
  done: {
    reply: "Done. The manual step you needed from me is complete.",
    toast: "Marked done — reply posts in 5s",
    zone: "done",
  },
  handledElsewhere: {
    reply: HANDLED_ELSEWHERE_REPLY,
    toast: "Handled elsewhere — reply posts in 5s",
    zone: "done",
  },
  noted: {
    reply: "Noted. No answer needed from me, carry on.",
    toast: "Noted — reply posts in 5s",
    zone: "done",
  },
  waiting: {
    reply: "I have this. I am waiting on someone else before I can respond.",
    toast: "Parked as waiting — reply posts in 5s",
    zone: "waiting",
  },
};

export function AttentionScreen() {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const homeFeedQuery = useHomeFeedQuery();
  const channelsQuery = useChannelsQuery();
  const { goChannel } = useAppNavigation();
  const { pendingIds, queueAction, queueBatch } = useActionQueue();
  // Held entries stay out of localStorage until commit (see hook docs).
  const {
    markDone,
    markResponded,
    markWaiting,
    restore,
    restoreEntry,
    zoneState,
  } = useAttentionZoneState(currentPubkey, pendingIds);
  const sendMutation = useSendMessageMutation(null, identityQuery.data);
  const { mutateAsync: sendMessage } = sendMutation;

  const feed = homeFeedQuery.data;
  const profilePubkeys = React.useMemo(() => {
    if (!feed) return [];
    return [
      ...new Set(
        [
          ...feed.feed.mentions,
          ...feed.feed.needsAction,
          ...feed.feed.activity,
          ...feed.feed.agentActivity,
        ]
          .map((item) => item.pubkey)
          // Include the viewer so the declared-ask tier can resolve the
          // viewer display name ("Needs <Name>, …") from the same lookup.
          .concat(currentPubkey ? [currentPubkey] : []),
      ),
    ];
  }, [feed, currentPubkey]);
  const profilesQuery = useUsersBatchQuery(profilePubkeys, {
    enabled: profilePubkeys.length > 0,
  });

  const inboxItems = React.useMemo(
    () =>
      buildInboxItems({
        channels: channelsQuery.data,
        currentPubkey,
        feed,
        profiles: profilesQuery.data?.profiles,
      }),
    [channelsQuery.data, currentPubkey, feed, profilesQuery.data?.profiles],
  );

  const { overrides, setOverride } = useBadgeOverrides(currentPubkey);

  const viewerProfile = currentPubkey
    ? profilesQuery.data?.profiles?.[currentPubkey.toLowerCase()]
    : undefined;
  const viewerName =
    viewerProfile?.displayName ?? viewerProfile?.name ?? undefined;

  const projection = React.useMemo(
    () =>
      projectAttention(inboxItems, zoneState, Math.floor(Date.now() / 1_000), {
        badgeOverrides: overrides,
        viewerName,
      }),
    [inboxItems, zoneState, overrides, viewerName],
  );

  // Snapshot for undo/revert closures: reading via ref avoids rebinding the
  // action handlers (and re-rendering every card) on each zone change.
  const zoneStateRef = React.useRef(zoneState);
  zoneStateRef.current = zoneState;

  const buildRevert = React.useCallback(
    (id: string) => {
      const previous = zoneStateRef.current[id];
      return () => restoreEntry(id, previous);
    },
    [restoreEntry],
  );

  const sendThreadReply = React.useCallback(
    async (item: AttentionItem, content: string) => {
      const channelId = item.inboxItem.item.channelId;
      if (!channelId) {
        throw new Error("This item has no source channel.");
      }
      const result = await sendMessage({
        channelId,
        content,
        parentEventId: item.inboxItem.item.id,
        mentionPubkeys: [item.inboxItem.item.pubkey],
      });
      markResponded(item.id);
      return result;
    },
    [markResponded, sendMessage],
  );

  const handleAction = React.useCallback(
    (item: AttentionItem, action: AttentionCardAction) => {
      // Nobody waits on a To note item: Noted there is local-only. It still
      // holds through the queue so every action type shares one code path;
      // the commit is a no-op send.
      if (!actionPublishes(item.askType, action)) {
        queueAction({
          itemId: item.id,
          toastLabel: "Noted locally",
          apply: () => markDone(item.id),
          revert: buildRevert(item.id),
          send: () => Promise.resolve(),
        });
        return;
      }
      const config = ACTION_CONFIG[action];
      queueAction({
        itemId: item.id,
        toastLabel: config.toast,
        apply: () =>
          config.zone === "waiting" ? markWaiting(item.id) : markDone(item.id),
        revert: buildRevert(item.id),
        send: () => sendThreadReply(item, config.reply),
      });
    },
    [buildRevert, markDone, markWaiting, queueAction, sendThreadReply],
  );

  const headsUpItems = projection.headsUp;
  const handleNoteAll = React.useCallback(() => {
    if (headsUpItems.length === 0) {
      return;
    }
    queueBatch({
      toastLabel:
        headsUpItems.length === 1
          ? "Noted 1 item locally"
          : `Noted ${headsUpItems.length} items locally`,
      items: headsUpItems.map((item) => ({
        itemId: item.id,
        apply: () => markDone(item.id),
        revert: buildRevert(item.id),
        send: () => Promise.resolve(),
      })),
    });
  }, [buildRevert, headsUpItems, markDone, queueBatch]);

  const handleReply = React.useCallback(
    (item: AttentionItem, text: string) => {
      queueAction({
        itemId: item.id,
        toastLabel: "Reply queued — posts in 5s",
        apply: () => markDone(item.id),
        revert: buildRevert(item.id),
        send: () => sendThreadReply(item, text),
      });
    },
    [buildRevert, markDone, queueAction, sendThreadReply],
  );

  const handleOpen = React.useCallback(
    (item: AttentionItem) => {
      const channelId = item.inboxItem.item.channelId;
      if (!channelId) return;
      void goChannel(channelId, {
        messageId: item.inboxItem.item.id,
        threadRootId: attentionThreadRootId(item.inboxItem),
      });
    },
    [goChannel],
  );

  return (
    <AttentionView
      errorMessage={
        homeFeedQuery.error !== null && homeFeedQuery.error !== undefined
          ? isRelayUnreachableError(homeFeedQuery.error)
            ? RELAY_UNREACHABLE_MESSAGE
            : homeFeedQuery.error instanceof Error
              ? homeFeedQuery.error.message
              : undefined
          : undefined
      }
      isLoading={homeFeedQuery.isLoading}
      onAction={handleAction}
      onNoteAll={handleNoteAll}
      onOpen={handleOpen}
      onOverrideBadge={setOverride}
      onReply={handleReply}
      onRestore={restore}
      pendingIds={pendingIds}
      projection={projection}
    />
  );
}

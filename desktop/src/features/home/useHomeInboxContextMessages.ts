import * as React from "react";

import type { InboxContextMessage, InboxItem } from "@/features/home/lib/inbox";
import {
  getReactionTargetId,
  toInboxContextMessage,
} from "@/features/home/lib/inboxViewHelpers";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel, RelayEvent } from "@/shared/api/types";
import {
  KIND_REACTION,
  KIND_STREAM_MESSAGE_EDIT,
} from "@/shared/constants/kinds";

type UseHomeInboxContextMessagesOptions = {
  channelMessages?: RelayEvent[];
  currentPubkey?: string;
  events: RelayEvent[];
  ownerProfiles?: UserProfileLookup;
  profiles?: UserProfileLookup;
  reactionEvents: RelayEvent[];
  relaySelfPubkey?: string | null;
  selectedChannel: Channel | null;
  selectedEventId: string | null;
  selectedItem: InboxItem | null;
};

export function useHomeInboxContextMessages({
  channelMessages,
  currentPubkey,
  events,
  ownerProfiles,
  profiles,
  reactionEvents,
  relaySelfPubkey,
  selectedChannel,
  selectedEventId,
  selectedItem,
}: UseHomeInboxContextMessagesOptions): InboxContextMessage[] {
  return React.useMemo(() => {
    if (!selectedItem) return [];

    const eventById = new Map(events.map((event) => [event.id, event]));
    const contextEventIds = new Set(eventById.keys());
    // Auxiliary overlays for the loaded context events: reactions AND edits.
    // Edits matter for surfaces especially — a card's live updates are edit
    // events, so without these the Inbox would render a stale card forever.
    const contextAuxiliaries = [
      ...(channelMessages ?? []),
      ...reactionEvents,
    ].filter((event) => {
      if (
        event.kind !== KIND_REACTION &&
        event.kind !== KIND_STREAM_MESSAGE_EDIT
      ) {
        return false;
      }
      const targetId = getReactionTargetId(event.tags);
      return Boolean(targetId && contextEventIds.has(targetId));
    });
    const currentUserAvatarUrl = currentPubkey
      ? (profiles?.[currentPubkey.toLowerCase()]?.avatarUrl ?? null)
      : null;
    const timelineMessages = formatTimelineMessages(
      [...events, ...contextAuxiliaries],
      selectedChannel,
      currentPubkey,
      currentUserAvatarUrl,
      profiles,
      undefined,
      undefined,
      undefined,
      relaySelfPubkey,
      ownerProfiles,
    );

    return timelineMessages.map((message) =>
      toInboxContextMessage(message, {
        eventById,
        fallbackAuthorPubkey: selectedItem.item.pubkey,
        profiles,
        selectedItemId: selectedEventId ?? selectedItem.id,
      }),
    );
  }, [
    channelMessages,
    currentPubkey,
    events,
    ownerProfiles,
    profiles,
    reactionEvents,
    relaySelfPubkey,
    selectedChannel,
    selectedEventId,
    selectedItem,
  ]);
}

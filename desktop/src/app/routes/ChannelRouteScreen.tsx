import * as React from "react";

import { getCachedSearchHitEvent } from "@/app/navigation/searchHitEventCache";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { ChannelScreen } from "@/features/channels/ui/ChannelScreen";
import { useProfileQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getEventById } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type ChannelRouteScreenProps = {
  channelId: string;
  selectedPostId: string | null;
  targetMessageId: string | null;
  targetReplyId: string | null;
  targetThreadRootId: string | null;
};

export function ChannelRouteScreen({
  channelId,
  selectedPostId,
  targetMessageId,
  targetReplyId,
  targetThreadRootId,
}: ChannelRouteScreenProps) {
  const { closeForumPost, goForumPost } = useAppNavigation();
  const channelsQuery = useChannelsQuery();
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const channels = channelsQuery.data ?? [];
  const activeChannel =
    channels.find((channel) => channel.id === channelId) ?? null;
  const [targetMessageEvents, setTargetMessageEvents] = React.useState<
    RelayEvent[]
  >(() => {
    const cachedTarget = getCachedSearchHitEvent(targetMessageId);
    return cachedTarget ? [cachedTarget] : [];
  });

  // Reset spliced target events when the channel context changes (channel
  // switch or entering/leaving a forum post). Tied to channel identity rather
  // than the route target so clearing the `messageId` param mid-channel keeps
  // the deep-linked row in view. Seeded with the mount key so the initial
  // cache-seeded events survive first commit; only a genuine channel change
  // clears them. Declared before the fetch effect so a channel switch clears
  // stale events before the new target is fetched.
  const previousResetKeyRef = React.useRef<string>(
    `${channelId}::${selectedPostId ?? ""}`,
  );
  React.useEffect(() => {
    const resetKey = `${channelId}::${selectedPostId ?? ""}`;
    if (previousResetKeyRef.current === resetKey) return;
    previousResetKeyRef.current = resetKey;
    setTargetMessageEvents([]);
  }, [channelId, selectedPostId]);

  React.useEffect(() => {
    let isCancelled = false;

    // Don't wipe already-spliced target events just because the route target
    // cleared (e.g. `onTargetReached` clears the `messageId` URL param once the
    // row is centered). In a channel whose feed doesn't already contain the
    // deep-linked message, the spliced event is the only copy — dropping it on
    // param-clear blanks the timeline. Resetting on channel / forum-post change
    // is handled by the effect below; here we only fetch when there's a target.
    if ((!targetMessageId && !targetThreadRootId) || selectedPostId) {
      return () => {
        isCancelled = true;
      };
    }

    const cachedTarget = getCachedSearchHitEvent(targetMessageId);
    if (cachedTarget) {
      setTargetMessageEvents((currentEvents) =>
        currentEvents.some((event) => event.id === cachedTarget.id)
          ? currentEvents
          : [...currentEvents, cachedTarget],
      );
    }

    const eventIds = [
      targetMessageId,
      targetThreadRootId && targetThreadRootId !== targetMessageId
        ? targetThreadRootId
        : null,
    ].filter((eventId): eventId is string => eventId !== null);

    void Promise.all(
      eventIds.map(async (eventId) => {
        try {
          return await getEventById(eventId);
        } catch (error) {
          console.error("Failed to load route event", eventId, error);
          return null;
        }
      }),
    ).then((events) => {
      if (!isCancelled) {
        setTargetMessageEvents((currentEvents) => {
          const fetchedEvents = events.filter(
            (event): event is RelayEvent => event !== null,
          );
          const eventsById = new Map<string, RelayEvent>();
          for (const event of [...currentEvents, ...fetchedEvents]) {
            eventsById.set(event.id, event);
          }
          return Array.from(eventsById.values());
        });
      }
    });

    return () => {
      isCancelled = true;
    };
  }, [selectedPostId, targetMessageId, targetThreadRootId]);

  if (channelsQuery.isPending && !activeChannel) {
    return (
      <ViewLoadingFallback
        includeHeader
        kind={selectedPostId ? "forum" : "channel"}
      />
    );
  }

  return (
    <ChannelScreen
      activeChannel={activeChannel}
      currentIdentity={identityQuery.data}
      currentProfile={profileQuery.data}
      onCloseForumPost={() => {
        void closeForumPost(channelId);
      }}
      onSelectForumPost={(postId) => {
        void goForumPost(channelId, postId);
      }}
      selectedForumPostId={selectedPostId}
      targetForumReplyId={targetReplyId}
      targetMessageEvents={targetMessageEvents}
      targetMessageId={targetMessageId}
    />
  );
}

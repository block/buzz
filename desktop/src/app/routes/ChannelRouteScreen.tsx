import * as React from "react";

import { getCachedSearchHitEvent } from "@/app/navigation/searchHitEventCache";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { ChannelScreen } from "@/features/channels/ui/ChannelScreen";
import { HuddleStartingView } from "@/features/huddle/components/HuddleStartingView";
import { huddleWindowChannelId } from "@/features/huddle/lib/huddleWindow";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import { useProfileQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getEventById } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type ChannelRouteScreenProps = {
  autoSendDraftKey: string | null;
  channelId: string;
  selectedPostId: string | null;
  targetMessageId: string | null;
  targetReplyId: string | null;
  targetThreadRootId: string | null;
  threadRailNavigation?: boolean;
};

const MAX_ROUTE_ANCESTOR_HOPS = 50;
const MAX_ROUTE_EVENT_CACHE = 500;
const routeEventCache = new Map<string, RelayEvent>();

function cacheRouteEvent(event: RelayEvent): void {
  routeEventCache.delete(event.id);
  routeEventCache.set(event.id, event);
  if (routeEventCache.size > MAX_ROUTE_EVENT_CACHE) {
    routeEventCache.delete(routeEventCache.keys().next().value as string);
  }
}

function getCachedRouteEvent(eventId: string): RelayEvent | null {
  return routeEventCache.get(eventId) ?? null;
}

async function fetchRouteEvent(eventId: string): Promise<RelayEvent | null> {
  const cached = getCachedRouteEvent(eventId);
  if (cached) return cached;
  try {
    const event = await getEventById(eventId);
    if (event) cacheRouteEvent(event);
    return event;
  } catch (error) {
    console.error("Failed to load route event", eventId, error);
    return null;
  }
}

function getReplyParentId(event: RelayEvent): string | null {
  if (isBroadcastReply(event.tags)) {
    return null;
  }

  return getThreadReference(event.tags).parentId;
}

async function fetchRouteTargetEvents(
  eventIds: string[],
  targetMessageId: string | null,
  targetThreadRootId: string | null,
): Promise<RelayEvent[]> {
  const eventsById = new Map<string, RelayEvent>();
  const addEvent = (event: RelayEvent | null) => {
    if (event) {
      eventsById.set(event.id, event);
    }
  };

  const uniqueEventIds = [...new Set(eventIds)];
  const initialEvents = await Promise.all(uniqueEventIds.map(fetchRouteEvent));
  for (const event of initialEvents) {
    addEvent(event);
  }

  const targetEvent = targetMessageId
    ? (eventsById.get(targetMessageId) ?? null)
    : null;
  if (!targetEvent) {
    return [...eventsById.values()];
  }

  const targetThreadRef = getThreadReference(targetEvent.tags);
  const threadRootId = targetThreadRootId ?? targetThreadRef.rootId ?? null;
  if (threadRootId && !eventsById.has(threadRootId)) {
    addEvent(await fetchRouteEvent(threadRootId));
  }

  let parentId = getReplyParentId(targetEvent);
  let guard = 0;
  while (
    parentId &&
    parentId !== threadRootId &&
    guard < MAX_ROUTE_ANCESTOR_HOPS
  ) {
    const parentEvent =
      eventsById.get(parentId) ?? (await fetchRouteEvent(parentId));
    if (!parentEvent) {
      break;
    }

    eventsById.set(parentEvent.id, parentEvent);
    parentId = getReplyParentId(parentEvent);
    guard += 1;
  }

  return [...eventsById.values()];
}

export function ChannelRouteScreen({
  autoSendDraftKey,
  channelId,
  selectedPostId,
  targetMessageId,
  targetReplyId,
  targetThreadRootId,
  threadRailNavigation = false,
}: ChannelRouteScreenProps) {
  const isHuddleTranscript = huddleWindowChannelId() !== null;
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
  const routeTargetKey = `${targetMessageId ?? ""}\u0000${targetThreadRootId ?? ""}`;
  const [routeLoadState, setRouteLoadState] = React.useState(() => ({
    key: routeTargetKey,
    settled:
      (!targetMessageId && !targetThreadRootId) || Boolean(selectedPostId),
  }));
  const targetMessageLoadSettled =
    routeLoadState.key === routeTargetKey && routeLoadState.settled;

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
    const shouldFetchTarget =
      Boolean(targetMessageId || targetThreadRootId) && !selectedPostId;
    setRouteLoadState({ key: routeTargetKey, settled: !shouldFetchTarget });

    // Don't wipe already-spliced target events just because the route target
    // cleared (e.g. `onTargetReached` clears the `messageId` URL param once the
    // row is centered). In a channel whose feed doesn't already contain the
    // deep-linked message, the spliced event is the only copy — dropping it on
    // param-clear blanks the timeline. Resetting on channel / forum-post change
    // is handled by the effect below; here we only fetch when there's a target.
    if (!shouldFetchTarget) {
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

    void fetchRouteTargetEvents(eventIds, targetMessageId, targetThreadRootId)
      .then((events) => {
        if (!isCancelled) {
          setTargetMessageEvents((currentEvents) => {
            const eventsById = new Map<string, RelayEvent>();
            for (const event of [...currentEvents, ...events]) {
              eventsById.set(event.id, event);
            }
            return Array.from(eventsById.values());
          });
        }
      })
      .finally(() => {
        if (!isCancelled) {
          setRouteLoadState({ key: routeTargetKey, settled: true });
        }
      });

    return () => {
      isCancelled = true;
    };
  }, [routeTargetKey, selectedPostId, targetMessageId, targetThreadRootId]);

  if (channelsQuery.isPending && !activeChannel) {
    if (isHuddleTranscript) {
      return <HuddleStartingView />;
    }
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
      autoSendDraftKey={autoSendDraftKey}
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
      targetMessageLoadSettled={targetMessageLoadSettled}
      threadRailNavigation={threadRailNavigation}
    />
  );
}

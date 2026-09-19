import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import type { SearchHighlightNavigation } from "@/app/navigation/searchHighlightNavigation";
import { getCachedSearchHitEvent } from "@/app/navigation/searchHitEventCache";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useOpenChannelDirectoryQuery } from "@/features/channels/openChannelDirectory";
import { ChannelScreen } from "@/features/channels/ui/ChannelScreen";
import { HuddleStartingView } from "@/features/huddle/components/HuddleStartingView";
import { huddleWindowChannelId } from "@/features/huddle/lib/huddleWindow";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import { useProfileQuery } from "@/features/profile/hooks";
import {
  useProjectHomeForChannelQuery,
  useProjectsQuery,
} from "@/features/projects/hooks";
import { findProjectHomeByChannelId } from "@/features/projects/lib/projectHomeChannel";
import {
  isProjectCollectionAuthoritative,
  isProjectRelayValidated,
  shouldUseScopedProjectHomeLookup,
} from "@/features/projects/projectSnapshot";
import { ProjectChannelHome } from "@/features/projects/ui/ProjectChannelHome";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getEventById } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

type ChannelRouteScreenProps = {
  autoSendDraftKey: string | null;
  channelId: string;
  searchHighlight: SearchHighlightNavigation | null | undefined;
  selectedPostId: string | null;
  targetMessageId: string | null;
  targetReplyId: string | null;
  targetThreadRootId: string | null;
};

const MAX_ROUTE_ANCESTOR_HOPS = 50;

async function fetchRouteEvent(eventId: string): Promise<RelayEvent | null> {
  try {
    return await getEventById(eventId);
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
  searchHighlight,
  selectedPostId,
  targetMessageId,
  targetReplyId,
  targetThreadRootId,
}: ChannelRouteScreenProps) {
  const isHuddleTranscript = huddleWindowChannelId() !== null;
  const queryClient = useQueryClient();
  const { closeForumPost, goForumPost } = useAppNavigation();
  const channelsQuery = useChannelsQuery();
  const projectsQuery = useProjectsQuery();
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const channels = channelsQuery.data ?? [];
  const memberChannel =
    channels.find((channel) => channel.id === channelId) ?? null;
  // A deep link to a non-member open channel resolves nothing in the
  // member-only poll list. Fall back to the discovery directory — but only for
  // that case, so a normal in-membership route never triggers the all-open
  // scan. React Query dedups the shared directory key across surfaces.
  const needsDirectoryFallback =
    !memberChannel && channelsQuery.isSuccess && !isHuddleTranscript;
  const openDirectoryQuery = useOpenChannelDirectoryQuery({
    enabled: needsDirectoryFallback,
  });
  const activeChannel =
    memberChannel ??
    openDirectoryQuery.data?.find((channel) => channel.id === channelId) ??
    null;
  const enumeratedProjectHome = findProjectHomeByChannelId(
    channelId,
    projectsQuery.data ?? [],
  );
  const projectCollectionIsAuthoritative =
    isProjectCollectionAuthoritative(queryClient);
  const projectHomeLookupQuery = useProjectHomeForChannelQuery(
    channelId,
    shouldUseScopedProjectHomeLookup({
      collectionIsAuthoritative: projectCollectionIsAuthoritative,
      hasEnumeratedProjectHome: Boolean(enumeratedProjectHome),
      isHuddleTranscript,
    }),
  );
  const projectHome =
    enumeratedProjectHome ?? projectHomeLookupQuery.data ?? null;
  const [targetMessageEvents, setTargetMessageEvents] = React.useState<
    RelayEvent[]
  >(() => {
    const cachedTarget = getCachedSearchHitEvent(targetMessageId);
    return cachedTarget ? [cachedTarget] : [];
  });
  const [activeSearchHighlight, setActiveSearchHighlight] =
    React.useState<SearchHighlightNavigation | null>(searchHighlight ?? null);
  const appliedSearchActivationIdRef = React.useRef<string | null>(
    searchHighlight?.activationId ?? null,
  );

  // Router state is transient and can be cleared by the target URL cleanup.
  // Retain the applied activation locally until an ordinary route transition
  // explicitly arrives without search state.
  React.useEffect(() => {
    if (searchHighlight === null) {
      appliedSearchActivationIdRef.current = null;
      setActiveSearchHighlight(null);
      return;
    }
    if (!searchHighlight) {
      const ordinaryTargetIds = [
        selectedPostId,
        targetMessageId,
        targetReplyId,
        targetThreadRootId,
      ].filter((targetId): targetId is string => targetId !== null);
      if (
        ordinaryTargetIds.length > 0 &&
        activeSearchHighlight &&
        !ordinaryTargetIds.includes(activeSearchHighlight.messageId)
      ) {
        appliedSearchActivationIdRef.current = null;
        setActiveSearchHighlight(null);
      }
      return;
    }
    if (appliedSearchActivationIdRef.current === searchHighlight.activationId) {
      return;
    }

    appliedSearchActivationIdRef.current = searchHighlight.activationId;
    setActiveSearchHighlight(searchHighlight);
  }, [
    activeSearchHighlight,
    searchHighlight,
    selectedPostId,
    targetMessageId,
    targetReplyId,
    targetThreadRootId,
  ]);

  // Reset spliced target events when the channel changes. Tied to channel
  // identity rather than the route target so clearing the `messageId` param
  // mid-channel keeps the deep-linked row in view. Seeded with the mount key so
  // the initial cache-seeded events survive first commit; only a genuine
  // channel change clears them. Declared before the fetch effect so a channel
  // switch clears stale events before the new target is fetched.
  const previousResetKeyRef = React.useRef<string>(channelId);
  React.useEffect(() => {
    if (previousResetKeyRef.current === channelId) return;
    previousResetKeyRef.current = channelId;
    appliedSearchActivationIdRef.current = null;
    setTargetMessageEvents([]);
    setActiveSearchHighlight(null);
  }, [channelId]);

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

    void fetchRouteTargetEvents(
      eventIds,
      targetMessageId,
      targetThreadRootId,
    ).then((events) => {
      if (!isCancelled) {
        setTargetMessageEvents((currentEvents) => {
          const eventsById = new Map<string, RelayEvent>();
          for (const event of [...currentEvents, ...events]) {
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

  if (
    !activeChannel &&
    (channelsQuery.isPending ||
      (needsDirectoryFallback && openDirectoryQuery.isPending))
  ) {
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

  if (projectHome && !isHuddleTranscript) {
    return (
      <ProjectChannelHome
        allowRepositoryHealing={isProjectRelayValidated(projectHome)}
        autoSendDraftKey={autoSendDraftKey}
        project={projectHome}
        projects={projectsQuery.data ?? [projectHome]}
        targetMessageEvents={targetMessageEvents}
        targetMessageId={targetMessageId}
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
      targetSearchMessageId={activeSearchHighlight?.messageId}
      targetSearchQuery={activeSearchHighlight?.query}
    />
  );
}

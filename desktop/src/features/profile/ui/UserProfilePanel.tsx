import * as React from "react";
import { ArrowLeft, X } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  useAgentMemoryQuery,
  useIsManagedAgent,
} from "@/features/agent-memory/hooks";
import { MemoryRefreshButton } from "@/features/agent-memory/ui/MemorySection";
import {
  useRelayAgentsQuery,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { useActiveAgentTurnsBridge } from "@/features/agents/activeAgentTurnsStore";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import { EditAgentDialog } from "@/features/agents/ui/EditAgentDialog";
import { useChannelsQuery } from "@/features/channels/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import {
  useContactListQuery,
  useFollowMutation,
  useProfileQuery,
  useUnfollowMutation,
  useUserProfileQuery,
} from "@/features/profile/hooks";
import {
  ChannelsFocusedView,
  MemoryFocusedView,
  ProfileSummaryView,
} from "@/features/profile/ui/UserProfilePanelSections";
import { useUserStatusQuery } from "@/features/user-status/hooks";
import { useAgentSession } from "@/shared/context/AgentSessionContext";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
  auxiliaryPanelContentPaddingClass,
} from "@/shared/layout/AuxiliaryPanelHeader";
import { cn } from "@/shared/lib/cn";
import type { Channel, ManagedAgent, RelayAgent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import {
  OverlayPanelBackdrop,
  PANEL_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
  PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";

type UserProfilePanelProps = {
  canResetWidth?: boolean;
  currentPubkey?: string;
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  onClose: () => void;
  onOpenDm?: (pubkeys: string[]) => void;
  onOpenProfile?: (pubkey: string) => void;
  onResetWidth?: () => void;
  onResizeStart?: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onViewChange: (
    view: ProfilePanelView,
    options?: { replace?: boolean },
  ) => void;
  pubkey: string;
  /**
   * When true, the panel sits beside a sibling pane managed by a single-panel
   * width controller (ChannelScreen). The width is clamped so the sibling keeps
   * at least THREAD_PANEL_MIN_WIDTH_PX. Standalone/floating mounts (e.g. Pulse)
   * have no such sibling, so they omit this and use the configured width
   * directly — otherwise `calc(100% - 300px)` would wrongly shrink the panel.
   */
  splitPaneClamp?: boolean;
  view: ProfilePanelView;
  widthPx: number;
};

export type ProfilePanelView = "summary" | "memories" | "channels";

const VIEW_TITLES: Record<ProfilePanelView, string> = {
  summary: "Profile",
  memories: "Memories",
  channels: "Channels",
};

function truncatePubkey(pubkey: string) {
  if (pubkey.length <= 16) {
    return pubkey;
  }

  return `${pubkey.slice(0, 8)}…${pubkey.slice(-8)}`;
}

type ProfileChannelLink = {
  id: string;
  name: string;
};

function deriveProfileChannels(
  pubkeyLower: string,
  relayAgent: RelayAgent | undefined,
  managedAgent: ManagedAgent | undefined,
  channels: Channel[] | undefined,
): ProfileChannelLink[] {
  const links = new Map<string, ProfileChannelLink>();
  const channelsByName = new Map(
    channels?.map((channel) => [channel.name, channel]) ?? [],
  );

  relayAgent?.channels.forEach((name, index) => {
    const channel = channelsByName.get(name);
    const id = relayAgent.channelIds[index] ?? channel?.id ?? name;
    links.set(id, { id, name });
  });

  if (managedAgent && channels) {
    for (const channel of channels) {
      const isMember = channel.memberPubkeys.some(
        (memberPubkey) => memberPubkey.toLowerCase() === pubkeyLower,
      );
      if (isMember) {
        links.set(channel.id, { id: channel.id, name: channel.name });
      }
    }
  }

  return [...links.values()].sort((left, right) =>
    left.name.localeCompare(right.name),
  );
}

export function UserProfilePanel({
  canResetWidth,
  currentPubkey,
  isSinglePanelView = false,
  layout = "standalone",
  onClose,
  onOpenDm,
  onOpenProfile,
  onResetWidth,
  onResizeStart,
  onViewChange,
  pubkey,
  splitPaneClamp = false,
  view,
  widthPx,
}: UserProfilePanelProps) {
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const isSplitLayout = layout === "split";
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const [editAgentOpen, setEditAgentOpen] = React.useState(false);

  const profileQuery = useUserProfileQuery(pubkey);
  const currentProfileQuery = useProfileQuery(currentPubkey !== undefined);

  // Batch avatar prefetch seeds kind:0 summaries without `about`; refetch on open
  // so the hero can show the full profile description from relay.
  React.useEffect(() => {
    void profileQuery.refetch();
  }, [profileQuery.refetch]);

  const relayAgentsQuery = useRelayAgentsQuery({ enabled: true });
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: true });
  const channelsQuery = useChannelsQuery();
  const presenceQuery = usePresenceQuery([pubkey]);
  const userStatusQuery = useUserStatusQuery([pubkey]);
  const contactListQuery = useContactListQuery(currentPubkey);
  const followMutation = useFollowMutation(currentPubkey);
  const unfollowMutation = useUnfollowMutation(currentPubkey);
  const { onOpenAgentSession } = useAgentSession();
  const { goChannel } = useAppNavigation();

  const profile = profileQuery.data;
  const ownerPubkey = profile?.ownerPubkey ?? null;
  const ownerProfileQuery = useUserProfileQuery(ownerPubkey ?? undefined);
  const pubkeyLower = pubkey.toLowerCase();
  const presenceStatus = presenceQuery.data?.[pubkeyLower];
  const userStatus = userStatusQuery.data?.[pubkeyLower];

  const relayAgent = relayAgentsQuery.data?.find(
    (agent) => agent.pubkey.toLowerCase() === pubkeyLower,
  );
  const managedAgent = managedAgentsQuery.data?.find(
    (agent) => agent.pubkey.toLowerCase() === pubkeyLower,
  );
  const isBot = Boolean(relayAgent || managedAgent);
  // Does THIS desktop hold the agent's seckey? Gates edit (which needs the key)
  // and grants owner access when the agent is managed locally.
  const isOwner = useIsManagedAgent(isBot ? pubkey : null);
  // Is the viewer the agent's declared owner (NIP-OA `ownerPubkey == me`)? This
  // is the right signal for viewing owner-scoped data (activity feed, memory):
  // the relay routes and the client decrypts those frames with the owner's OWN
  // key, so the agent's seckey is never needed. Computed here (before the gates
  // that consume it) so visibility keys off declared ownership, not key custody.
  const isCurrentUserOwner =
    currentPubkey !== undefined &&
    ownerPubkey !== null &&
    ownerPubkey.toLowerCase() === currentPubkey.toLowerCase();
  // The viewer may see owner-scoped data if they declared-own the agent OR they
  // manage it locally (older agents may not advertise an owner pubkey). Every
  // real boundary is server-side, so this only controls what UI we paint.
  const viewerIsOwner = isCurrentUserOwner || isOwner === true;

  // Populate the active-turns store for this agent so useActiveAgentTurns works
  // even if the Agents page hasn't been visited yet.
  const bridgeAgents = React.useMemo(
    () =>
      managedAgent
        ? [{ pubkey: managedAgent.pubkey, status: managedAgent.status }]
        : [],
    [managedAgent],
  );
  // The observer bridge subscribes on the OWNER's own pubkey and decrypts the
  // agent's telemetry with the owner's key — no agent seckey needed. It only
  // decrypts frames whose agent pubkey is "known", and only subscribes when an
  // agent is running/deployed. For a remote agent we own but don't manage
  // locally, `managedAgent` is undefined, so we seed the bridge from the relay
  // agent (treated as "deployed") when the viewer is the declared owner. This
  // mirrors what the composer-area ingress already does in ChannelScreen.
  const observerBridgeAgents = React.useMemo(() => {
    if (managedAgent) {
      return [{ pubkey: managedAgent.pubkey, status: managedAgent.status }];
    }
    if (viewerIsOwner && relayAgent) {
      return [
        {
          pubkey: relayAgent.pubkey,
          status: "deployed" as ManagedAgent["status"],
        },
      ];
    }
    return [];
  }, [managedAgent, relayAgent, viewerIsOwner]);
  useActiveAgentTurnsBridge(bridgeAgents);
  useManagedAgentObserverBridge(observerBridgeAgents);
  const canEditAgent = isOwner === true && managedAgent !== undefined;
  const memoryQuery = useAgentMemoryQuery(pubkey, {
    enabled: viewerIsOwner,
  });
  const isSelf =
    currentPubkey !== undefined && pubkeyLower === currentPubkey.toLowerCase();
  const canViewActivity = viewerIsOwner && Boolean(onOpenAgentSession);
  const isFollowing =
    !isSelf &&
    (contactListQuery.data?.contacts.some(
      (contact) => contact.pubkey.toLowerCase() === pubkeyLower,
    ) ??
      false);

  const profileChannels = React.useMemo(
    () =>
      deriveProfileChannels(
        pubkeyLower,
        relayAgent,
        managedAgent,
        channelsQuery.data,
      ),
    [pubkeyLower, relayAgent, managedAgent, channelsQuery.data],
  );

  const channelIdToName = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const channel of channelsQuery.data ?? []) {
      map[channel.id] = channel.name;
    }
    return map;
  }, [channelsQuery.data]);

  const handleMessage = React.useCallback(() => {
    onOpenDm?.([pubkey]);
    onClose();
  }, [onClose, onOpenDm, pubkey]);

  const handleEditAgent = React.useCallback(() => {
    setEditAgentOpen(true);
  }, []);

  const handleOpenActivity = React.useCallback(() => {
    onClose();
    onOpenAgentSession?.(pubkey);
  }, [onClose, onOpenAgentSession, pubkey]);

  const handleOpenChannel = React.useCallback(
    (channelId: string) => {
      void goChannel(channelId);
    },
    [goChannel],
  );

  const displayName = profile?.displayName ?? truncatePubkey(pubkey);
  const ownerHandle = React.useMemo(() => {
    if (ownerPubkey) {
      const ownerProfile = ownerProfileQuery.data;
      return (
        ownerProfile?.nip05Handle?.trim() ||
        ownerProfile?.displayName?.trim() ||
        truncatePubkey(ownerPubkey)
      );
    }

    if (currentPubkey === undefined || isOwner !== true) {
      return null;
    }

    const currentProfile = currentProfileQuery.data;
    return (
      currentProfile?.nip05Handle?.trim() ||
      currentProfile?.displayName?.trim() ||
      truncatePubkey(currentPubkey)
    );
  }, [
    currentProfileQuery.data,
    currentPubkey,
    isOwner,
    ownerProfileQuery.data,
    ownerPubkey,
  ]);
  const ownerDisplayName = ownerHandle
    ? isCurrentUserOwner || (!ownerPubkey && isOwner === true)
      ? `${ownerHandle} (you)`
      : ownerHandle
    : null;
  const panelTitle = VIEW_TITLES[view];
  const memoryCount = memoryQuery.data
    ? (memoryQuery.data.core ? 1 : 0) + memoryQuery.data.memories.length
    : undefined;

  const headerLeftContent = (
    <AuxiliaryPanelHeaderGroup>
      {view !== "summary" ? (
        <Button
          aria-label="Back to profile"
          className="shrink-0"
          data-testid="user-profile-panel-back"
          onClick={() => onViewChange("summary")}
          size="icon"
          type="button"
          variant="outline"
        >
          <ArrowLeft />
        </Button>
      ) : null}
      <AuxiliaryPanelTitle>{panelTitle}</AuxiliaryPanelTitle>
    </AuxiliaryPanelHeaderGroup>
  );

  const headerActions = (
    <div className="ml-auto flex shrink-0 items-center gap-2">
      {view === "memories" && isOwner === true ? (
        <MemoryRefreshButton agentPubkey={pubkey} variant="outline" />
      ) : null}
      <Button
        aria-label="Close profile"
        data-testid="user-profile-panel-close"
        onClick={onClose}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X />
      </Button>
    </div>
  );

  const profileBody = (
    <div
      className={cn(
        "min-h-0 flex-1 overflow-y-auto px-4 pb-6",
        isSplitLayout && auxiliaryPanelContentPaddingClass,
        !isSplitLayout && !isFloatingOverlay && "pt-[3.25rem]",
      )}
    >
      {view === "summary" ? (
        <ProfileSummaryView
          canEditAgent={canEditAgent}
          canViewActivity={canViewActivity}
          channelCount={profileChannels.length}
          channelIdToName={channelIdToName}
          channelsLoading={channelsQuery.isLoading}
          displayName={displayName}
          followMutation={followMutation}
          handleEditAgent={handleEditAgent}
          handleMessage={handleMessage}
          handleOpenActivity={handleOpenActivity}
          isBot={isBot}
          isFollowing={isFollowing}
          isOwner={viewerIsOwner}
          isSelf={isSelf}
          managedAgent={managedAgent}
          memoriesLoading={memoryQuery.isLoading}
          memoryCount={memoryCount}
          ownerDisplayName={ownerDisplayName}
          ownerAvatarUrl={ownerProfileQuery.data?.avatarUrl ?? null}
          ownerHandle={ownerHandle}
          ownerPubkey={ownerPubkey}
          onOpenChannels={() => onViewChange("channels")}
          onOpenOwner={
            ownerPubkey && onOpenProfile
              ? () => onOpenProfile(ownerPubkey)
              : undefined
          }
          onOpenMemories={() => onViewChange("memories")}
          onOpenDm={onOpenDm}
          presenceLoaded={presenceQuery.isSuccess}
          presenceStatus={presenceStatus}
          profile={profile}
          pubkey={pubkey}
          relayAgent={relayAgent}
          unfollowMutation={unfollowMutation}
          userStatus={userStatus}
        />
      ) : null}

      {view === "memories" ? (
        <MemoryFocusedView agentPubkey={pubkey} isOwner={viewerIsOwner} />
      ) : null}

      {view === "channels" ? (
        <ChannelsFocusedView
          channels={profileChannels}
          isLoading={channelsQuery.isLoading}
          onOpenChannel={handleOpenChannel}
        />
      ) : null}
    </div>
  );

  const editAgentDialog =
    canEditAgent && managedAgent ? (
      <EditAgentDialog
        agent={managedAgent}
        onOpenChange={setEditAgentOpen}
        open={editAgentOpen}
      />
    ) : null;

  if (isSplitLayout) {
    return (
      <>
        <div className="flex min-h-0 flex-1 flex-col">
          <AuxiliaryPanelHeader>
            {headerLeftContent}
            {headerActions}
          </AuxiliaryPanelHeader>
          {profileBody}
        </div>
        {editAgentDialog}
      </>
    );
  }

  return (
    <>
      {isFloatingOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_BASE_CLASS,
          isSinglePanelView && "border-l-0",
          isFloatingOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="user-profile-panel"
        style={{
          width: isSinglePanelView
            ? "100%"
            : splitPaneClamp
              ? `min(${widthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`
              : `${widthPx}px`,
        }}
      >
        {!isOverlay && !isSinglePanelView && onResizeStart && (
          <button
            aria-label="Resize profile panel"
            className="peer/profile-resize group/profile-resize absolute inset-y-0 left-0 z-40 w-3 -translate-x-1/2 cursor-col-resize"
            data-testid="user-profile-resize-handle"
            onDoubleClick={canResetWidth ? onResetWidth : undefined}
            onPointerDown={onResizeStart}
            title={
              canResetWidth
                ? "Drag to resize. Double-click to reset width."
                : "Drag to resize."
            }
            type="button"
          >
            <span className="absolute bottom-0 left-1/2 top-10 w-px -translate-x-1/2 bg-transparent transition-colors group-hover/profile-resize:bg-border/80 group-focus-visible/profile-resize:bg-border/80" />
          </button>
        )}

        {!isOverlay ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 top-0 z-40 h-[3.25rem] bg-background/80 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55"
          />
        ) : null}

        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-[3.25rem] min-h-[3.25rem] shrink-0 gap-2.5 bg-transparent px-4 py-2 sm:pl-6 sm:pr-3`
              : isOverlay
                ? "relative z-50 min-h-[3.25rem] shrink-0 gap-3 bg-background/80 px-5 py-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55"
                : "absolute inset-x-0 top-0 z-50 min-h-[3.25rem] gap-3 bg-transparent px-3 py-2 after:absolute after:bottom-0 after:-left-px after:top-0 after:w-px after:bg-border/45 after:transition-colors peer-hover/profile-resize:after:bg-border/80 peer-focus-visible/profile-resize:after:bg-border/80",
          )}
          data-tauri-drag-region
        >
          {headerLeftContent}
          {headerActions}
        </div>

        {profileBody}
      </aside>
      {editAgentDialog}
    </>
  );
}

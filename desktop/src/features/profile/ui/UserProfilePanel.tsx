import * as React from "react";
import { ArrowLeft, X } from "lucide-react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { MemoryRefreshButton } from "@/features/agent-memory/ui/MemorySection";
import {
  useRelayAgentsQuery,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  useArchiveIdentityMutation,
  useIsIdentityArchived,
  useOaOwnerQuery,
  useUnarchiveIdentityMutation,
} from "@/features/identity-archive/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import {
  useContactListQuery,
  useFollowMutation,
  useUnfollowMutation,
  useUserProfileQuery,
} from "@/features/profile/hooks";
import {
  ChannelsFocusedView,
  MemoryFocusedView,
  ProfileSummaryView,
} from "@/features/profile/ui/UserProfilePanelSections";
import { useMyRelayMembershipQuery } from "@/features/relay-members/hooks";
import { useUserStatusQuery } from "@/features/user-status/hooks";
import { useAgentSession } from "@/shared/context/AgentSessionContext";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
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
  canResetWidth: boolean;
  currentPubkey?: string;
  isSinglePanelView?: boolean;
  onClose: () => void;
  onOpenDm?: (pubkeys: string[]) => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  pubkey: string;
  splitPaneClamp?: boolean;
  widthPx: number;
};

type ProfilePanelView = "summary" | "memories" | "channels";

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
  onClose,
  onOpenDm,
  onResetWidth,
  onResizeStart,
  pubkey,
  splitPaneClamp = false,
  widthPx,
}: UserProfilePanelProps) {
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const usesChannelSplitChrome =
    splitPaneClamp && !isOverlay && !isSinglePanelView;
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const [view, setView] = React.useState<ProfilePanelView>("summary");

  const profileQuery = useUserProfileQuery(pubkey);

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
  const myMembershipQuery = useMyRelayMembershipQuery();
  const oaOwnerQuery = useOaOwnerQuery(
    pubkey,
    currentPubkey !== undefined &&
      pubkey.toLowerCase() !== currentPubkey.toLowerCase(),
  );
  const isArchived = useIsIdentityArchived(pubkey);
  const contactListQuery = useContactListQuery(currentPubkey);
  const followMutation = useFollowMutation(currentPubkey);
  const unfollowMutation = useUnfollowMutation(currentPubkey);
  const archiveMutation = useArchiveIdentityMutation();
  const unarchiveMutation = useUnarchiveIdentityMutation();
  const { onOpenAgentSession } = useAgentSession();
  const { goChannel } = useAppNavigation();

  const profile = profileQuery.data;
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
  const isOwner = useIsManagedAgent(isBot ? pubkey : null);
  const isSelf =
    currentPubkey !== undefined && pubkeyLower === currentPubkey.toLowerCase();
  const canViewActivity = isOwner === true && Boolean(onOpenAgentSession);
  const isFollowing =
    !isSelf &&
    (contactListQuery.data?.contacts.some(
      (contact) => contact.pubkey.toLowerCase() === pubkeyLower,
    ) ??
      false);

  const myRole = myMembershipQuery.data?.role;
  const isRelayAdminOrOwner = myRole === "owner" || myRole === "admin";
  const isOaOwnerOfViewee = oaOwnerQuery.data?.isMe === true;
  const canArchive = isSelf || isRelayAdminOrOwner || isOaOwnerOfViewee;

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

  const prevPubkeyRef = React.useRef(pubkey);
  if (prevPubkeyRef.current !== pubkey) {
    prevPubkeyRef.current = pubkey;
    setView("summary");
  }

  const handleArchive = React.useCallback(() => {
    archiveMutation.mutate(
      { targetPubkey: pubkey },
      {
        onSuccess: () => toast.success("Archived on this relay"),
        onError: (error) =>
          toast.error(
            `Archive failed: ${error instanceof Error ? error.message : String(error)}`,
          ),
      },
    );
  }, [archiveMutation, pubkey]);

  const handleUnarchive = React.useCallback(() => {
    unarchiveMutation.mutate(
      { targetPubkey: pubkey },
      {
        onSuccess: () => toast.success("Unarchived on this relay"),
        onError: (error) =>
          toast.error(
            `Unarchive failed: ${error instanceof Error ? error.message : String(error)}`,
          ),
      },
    );
  }, [pubkey, unarchiveMutation]);

  const handleMessage = React.useCallback(() => {
    onOpenDm?.([pubkey]);
    onClose();
  }, [onClose, onOpenDm, pubkey]);

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
  const panelTitle = VIEW_TITLES[view];

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
        {!isOverlay && !isSinglePanelView && (
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
            className={cn(
              "pointer-events-none absolute inset-x-0 top-0 z-40 bg-background/80 backdrop-blur-md after:absolute after:left-0 after:right-0 after:top-10 after:h-px after:bg-border/35 supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55",
              usesChannelSplitChrome ? "h-[92px]" : "h-[76px]",
            )}
          />
        ) : null}

        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-[76px] min-h-[76px] shrink-0 gap-[10px] bg-transparent pb-[4px] pl-[16px] pr-[8px] pt-[42px] sm:pl-[24px] sm:pr-[12px]`
              : isOverlay
                ? "relative z-50 min-h-[44px] shrink-0 gap-3 bg-background/80 px-3 py-[6px] backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55"
                : cn(
                    "absolute inset-x-0 z-50 bg-transparent after:absolute after:bottom-0 after:-left-px after:top-0 after:w-px after:bg-border/45 after:transition-colors peer-hover/profile-resize:after:bg-border/80 peer-focus-visible/profile-resize:after:bg-border/80",
                    usesChannelSplitChrome
                      ? "top-[48px] h-[32px] gap-[10px] py-0 pl-[16px] pr-[8px] sm:pr-[12px]"
                      : "top-[42px] min-h-[32px] gap-3 px-3 py-[4px]",
                  ),
          )}
          data-tauri-drag-region
        >
          <div className="flex min-w-0 items-center gap-1.5">
            {view !== "summary" ? (
              <Button
                aria-label="Back to profile"
                className={cn(
                  "shrink-0",
                  usesChannelSplitChrome
                    ? "h-8 w-8 rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-4"
                    : "h-7 w-7 rounded-full text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                )}
                data-testid="user-profile-panel-back"
                onClick={() => setView("summary")}
                size="icon"
                type="button"
                variant="ghost"
              >
                <ArrowLeft
                  className={cn(usesChannelSplitChrome ? "size-4" : "size-3.5")}
                />
              </Button>
            ) : null}
            <h2
              className={cn(
                "translate-y-px font-semibold tracking-tight",
                usesChannelSplitChrome
                  ? "text-base leading-6"
                  : "text-sm leading-5",
              )}
            >
              {panelTitle}
            </h2>
          </div>
          <div className="ml-auto flex shrink-0 items-center gap-2">
            {view === "memories" && isOwner === true ? (
              <MemoryRefreshButton
                agentPubkey={pubkey}
                className={cn(
                  usesChannelSplitChrome
                    ? "h-8 w-8 rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-4"
                    : "h-4 w-4 rounded-full text-foreground hover:bg-muted/60 hover:text-foreground",
                )}
                iconClassName={cn(
                  usesChannelSplitChrome ? "size-4" : "h-2.5 w-2.5",
                )}
              />
            ) : null}
            <Button
              aria-label="Close profile"
              className={cn(
                usesChannelSplitChrome
                  ? "h-8 w-8 rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-5"
                  : "h-4 w-4 rounded-full text-foreground hover:bg-muted/60 hover:text-foreground",
              )}
              data-testid="user-profile-panel-close"
              onClick={onClose}
              size="icon"
              type="button"
              variant="ghost"
            >
              <X
                className={cn(
                  usesChannelSplitChrome ? "size-5" : "h-2.5 w-2.5",
                )}
              />
            </Button>
          </div>
        </div>

        <div
          className={cn(
            "min-h-0 flex-1 overflow-y-auto px-4 pb-6",
            !isFloatingOverlay &&
              (usesChannelSplitChrome ? "pt-[92px]" : "pt-[76px]"),
          )}
        >
          {view === "summary" ? (
            <ProfileSummaryView
              canArchive={canArchive}
              canViewActivity={canViewActivity}
              channelCount={profileChannels.length}
              channelsLoading={channelsQuery.isLoading}
              displayName={displayName}
              followMutation={followMutation}
              handleArchive={handleArchive}
              handleMessage={handleMessage}
              handleOpenActivity={handleOpenActivity}
              handleUnarchive={handleUnarchive}
              isArchived={isArchived}
              isBot={isBot}
              isFollowing={isFollowing}
              isOwner={isOwner}
              isSelf={isSelf}
              managedAgent={managedAgent}
              onOpenChannels={() => setView("channels")}
              onOpenMemories={() => setView("memories")}
              onOpenDm={onOpenDm}
              presenceStatus={presenceStatus}
              profile={profile}
              pubkey={pubkey}
              relayAgent={relayAgent}
              unfollowMutation={unfollowMutation}
              userStatus={userStatus}
              archiveMutation={archiveMutation}
              unarchiveMutation={unarchiveMutation}
            />
          ) : null}

          {view === "memories" ? (
            <MemoryFocusedView agentPubkey={pubkey} isOwner={isOwner} />
          ) : null}

          {view === "channels" ? (
            <ChannelsFocusedView
              channels={profileChannels}
              isLoading={channelsQuery.isLoading}
              onOpenChannel={handleOpenChannel}
            />
          ) : null}
        </div>
      </aside>
    </>
  );
}

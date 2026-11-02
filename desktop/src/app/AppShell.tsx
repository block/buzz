import * as React from "react";
import { ProtectedGlobalOverlay } from "@protected-feature-components";
import { useQueryClient } from "@tanstack/react-query";
import { Outlet, useLocation } from "@tanstack/react-router";
import { deriveShellRoute, markAllReadSources } from "@/app/AppShell.helpers";
import { useTerminalContext } from "@/app/useTerminalContext";
import { AppShellProvider } from "@/app/AppShellContext";
import { AppShellOverlays, TerminalBootstrap } from "@/app/AppShellOverlays";
import { AppShellChannelSurface } from "@/app/AppShellChannelSurface";
import { AppShellFrame } from "@/app/AppShellFrame";
import { AppTopChrome } from "@/app/AppTopChrome";
import { EditCommunityDialog } from "@/features/communities/ui/EditCommunityDialog";
import { MobileWorkspaceHeader } from "@/features/communities/ui/MobileWorkspaceHeader";
import { useIsMobile } from "@/shared/hooks/use-mobile";
import {
  type TerminalContextOverride,
  TerminalContextOverrideProvider,
} from "@/app/TerminalContextOverrideContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useBackForwardControls } from "@/app/navigation/useBackForwardControls";
import { useCommunityNavigationTransitions } from "@/app/useCommunityNavigationTransitions";
import { useChannelBrowserDialog } from "@/app/useChannelBrowserDialog";
import { useMarkAsReadShortcuts } from "@/app/useMarkAsReadShortcuts";
import { useSettingsShortcuts } from "@/app/useSettingsShortcuts";
import { useAppShellKeyboardShortcuts } from "@/app/useAppShellKeyboardShortcuts";
import { listenForChannelSearchRequests } from "@/shared/lib/channelSearchRequest";
import { useAppShellDesktopNotifications } from "@/app/useAppShellDesktopNotifications";
import { useAppShellLifecycleEffects } from "@/app/useAppShellLifecycleEffects";
import { useChannelActivityProjection } from "@/app/useChannelActivityProjection";
import { useTauriWindowDrag } from "@/app/useTauriWindowDrag";
import { useWebviewZoomShortcuts } from "@/app/useWebviewZoomShortcuts";
import {
  channelsQueryKey,
  useChannelsQuery,
  useCreateChannelMutation,
  useHideDmMutation,
  useOpenDmMutation,
} from "@/features/channels/hooks";
import { useDmResurfaceFromMessages } from "@/features/channels/useDmResurfaceFromMessages";
import { useUnreadChannels } from "@/features/channels/useUnreadChannels";
import { useMembershipNotifications } from "@/features/channels/useMembershipNotifications";
import { useFeedItemState } from "@/features/home/useFeedItemState";
import { useThreadFollows } from "@/features/messages/lib/useThreadFollows";
import {
  useHomeFeedNotifications,
  useHomeFeedNotificationState,
} from "@/features/notifications/hooks";
import { PreventSleepProvider } from "@/features/agents/usePreventSleep";
import { requestOpenCreateAgent } from "@/features/agents/openCreateAgentEvent";
import { useAgentsDataRefresh } from "@/features/agents/lib/useAgentsDataRefresh";
import { useManagedAgentRuntimeReconciliation } from "@/features/agents/useManagedAgentRuntimeReconciliation";
import { useAutoRestartPolicy } from "@/features/agents/lib/useAutoRestartPolicy";
import { useMentionReconciler } from "@/features/messages/ui/useMentionReconciler";
import { usePersonaSync } from "@/features/agents/lib/usePersonaSync";
import { useAgentObserverIngestion } from "@/features/agents/useAgentObserverIngestion";
import { AgentManagementDialogs } from "@/features/agents/ui/AgentManagementDialogs";
import { RequestedAgentCreateDialogs } from "@/features/agents/ui/RequestedAgentCreateDialogs";
import {
  usePresenceSession,
  usePresenceSubscription,
} from "@/features/presence/hooks";
import {
  useSetUserStatusMutation,
  useUserStatusQuery,
  visibleUserStatus,
  useUserStatusSubscription,
} from "@/features/user-status/hooks";
import { useCommunityEmojiLiveUpdates } from "@/features/custom-emoji/hooks";
import { useArchiveSync } from "@/features/local-archive/useArchiveSync";
import { useArchiveAgentMetricsBridge } from "@/features/local-archive/useArchiveAgentMetricsBridge";
import { useObserverArchiveReconciliation } from "@/features/local-archive/useObserverArchiveSeed";
import { useAgentMetricArchiveSeed } from "@/features/local-archive/useAgentMetricArchiveSeed";
import { useProfileQuery } from "@/features/profile/hooks";
import { SendFeedbackController } from "@/features/settings/ui/SendFeedbackController";
import {
  DEFAULT_SETTINGS_SECTION,
  type SettingsSection,
  isSettingsSection,
} from "@/features/settings/ui/SettingsPanels";
import { AppSidebar } from "@/features/sidebar/ui/AppSidebar";
import "@/features/settings/ui/AppearanceWorkspaceSelection.css";
import { requestFocusedThreadClose } from "@/features/channels/focusedThreadCloseRequest";
import { CommunityRail } from "@/features/sidebar/ui/CommunityRail";
import { useChannelMutes } from "@/features/sidebar/lib/useChannelMutes";
import { useChannelStars } from "@/features/sidebar/lib/useChannelStars";
import { useCommunities } from "@/features/communities/useCommunities";
import { useWorkspaceSwitchShortcuts } from "@/features/communities/useWorkspaceSwitchShortcuts";
import type { Community } from "@/features/communities/types";
import { useMembershipDenialWatcher } from "@/features/communities/useMembershipDenialWatcher";
import { plainRelayErrorMessage } from "@/shared/lib/relayError";
import {
  consumePendingCommunityRestore,
  loadCommunityDestination,
  saveCommunityDestination,
} from "@/features/communities/communityNavigationStorage";
import { useAddCommunityDialogState } from "@/features/communities/addCommunityPrefill";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useRelayAutoHeal } from "@/shared/api/useRelayAutoHeal";
import { useDeferredStartup } from "@/shared/hooks/useDeferredStartup";
import { useWebviewScrollBoundaryLock } from "@/shared/hooks/useWebviewScrollBoundaryLock";
import { joinChannel } from "@/shared/api/tauri";
import type { ChannelVisibility, SearchHit } from "@/shared/api/types";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { ChannelNavigationProvider } from "@/shared/context/ChannelNavigationContext";
import { useAppDeepLinks } from "@/shared/useAppDeepLinks";
import { SidebarProvider } from "@/shared/ui/sidebar";
import { RelayConnectionOverlay } from "@/app/RelayConnectionOverlay";
import { useSidebarRelayConnectionCard } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { AppShellTrayMenu } from "@/app/useAppShellTrayMenu";
import { AppProfilePanelProvider } from "@/app/AppProfilePanelProvider";
import { LazySettingsScreen } from "@/app/LazySettingsScreen";
export function AppShell() {
  useWebviewZoomShortcuts();
  useTauriWindowDrag();
  useWebviewScrollBoundaryLock();
  const communitiesHook = useCommunities();
  const location = useLocation();
  const isNarrowWindow = useIsMobile();
  const { selectedChannelId, selectedView } = React.useMemo(
    () => deriveShellRoute(location.pathname),
    [location.pathname],
  );
  const hasCommunityRail =
    communitiesHook.communities.length > 1 && !isNarrowWindow;
  const addCommunityDialog = useAddCommunityDialogState();
  const [workspaceDialogMode, setWorkspaceDialogMode] = React.useState<
    "add-community" | "create" | "join"
  >("add-community");
  const [workspaceSettingsCommunity, setWorkspaceSettingsCommunity] =
    React.useState<Community | null>(null);
  const openWorkspaceDialog = (mode: "create" | "join") => {
    setWorkspaceDialogMode(mode);
    addCommunityDialog.openDialog();
  };
  const openWorkspaceSettings = React.useCallback(() => {
    if (communitiesHook.activeCommunity) {
      setWorkspaceSettingsCommunity(communitiesHook.activeCommunity);
    }
  }, [communitiesHook.activeCommunity]);
  const handleAddCommunityDialogOpenChange = (open: boolean) => {
    if (!open) setWorkspaceDialogMode("add-community");
    addCommunityDialog.onOpenChange(open);
  };
  const [isChannelManagementOpen, setIsChannelManagementOpen] =
    React.useState(false);
  const [managedChannelId, setManagedChannelId] = React.useState<string | null>(
    null,
  );
  const [searchFocusRequest, setSearchFocusRequest] = React.useState(0);
  const [scopeSearchFocusRequest, setScopeSearchFocusRequest] =
    React.useState(0);
  const [isCreateChannelOpen, setIsCreateChannelOpen] = React.useState(false);
  const [isSendFeedbackOpen, setIsSendFeedbackOpen] = React.useState(false);
  const mainInsetRef = React.useRef<HTMLElement>(null);
  const queryClient = useQueryClient();
  useManagedAgentRuntimeReconciliation(communitiesHook.communities); // sync storage snapshot
  const {
    goChannel,
    goHome,
    goPower,
    goNewMessage,
    goAgents,
    goTeam,
    goClients,
    goWork,
    goToday,
    goSettings,
    closeSettings,
    openSearchHit,
  } = useAppNavigation();
  const handleSidebarChannelSelect = React.useCallback(
    (channelId: string) => {
      void goChannel(channelId);
    },
    [goChannel],
  );
  const { canGoBack, canGoForward, goBack, goForward } =
    useBackForwardControls();
  const {
    removeCommunity: handleRemoveCommunity,
    switchCommunity: handleSwitchCommunity,
  } = useCommunityNavigationTransitions({
    communities: communitiesHook,
    goHome,
    selectedChannelId,
    selectedView,
  });
  useWorkspaceSwitchShortcuts(
    communitiesHook.communities,
    communitiesHook.activeCommunity?.id ?? null,
    handleSwitchCommunity,
  );
  // Settings lives in history so back returns to the previous app entry.
  const settingsOpen = location.pathname === "/settings";
  const showAppTopChrome =
    !settingsOpen &&
    selectedView !== "pins" &&
    // Activity draws its own header; on a narrow window the sidebar drawer's
    // only handle is the shell bar, so it stays there.
    (location.pathname !== "/" || isNarrowWindow) &&
    location.pathname !== "/messages/new" &&
    selectedView !== "channel";
  const locationSearchSection = (location.search as { section?: unknown })
    .section;
  const settingsSection: SettingsSection = isSettingsSection(
    locationSearchSection,
  )
    ? locationSearchSection
    : DEFAULT_SETTINGS_SECTION;
  const startupReady = useDeferredStartup();
  const identityQuery = useIdentityQuery();
  const { mutedChannelIds, muteChannel, unmuteChannel } = useChannelMutes(
    identityQuery.data?.pubkey,
    communitiesHook.activeCommunity?.relayUrl,
  );
  const { starredChannelIds, starChannel, unstarChannel } = useChannelStars(
    identityQuery.data?.pubkey,
    communitiesHook.activeCommunity?.relayUrl,
  );
  usePersonaSync(
    identityQuery.data?.pubkey,
    communitiesHook.activeCommunity?.relayUrl,
  );
  useAgentsDataRefresh();
  // Chunk F: auto-restart drifted idle agents (per-agent opt-out, default ON).
  useAutoRestartPolicy();
  // The relay-side check that no mention to a local agent goes unanswered,
  // whatever the send path did (GATE-303), and the catch-up at launch.
  useMentionReconciler();
  // Owner-global observer ingestion: receives + decrypts agent observer
  // frames and keeps derived active-turn liveness in sync app-wide, so no
  // individual screen/panel has to mount its own bridge for ingestion.
  // Intentionally mounted without a `startupReady`/identity guard: before
  // `currentPubkey` resolves the hook ingests managed agents only, and
  // relay-owned agents join automatically once identity arrives. Adding a
  // guard here would drop managed-agent coverage during startup.
  useAgentObserverIngestion();
  // Kind 24200 is relay-ephemeral, so reconciliation runs eagerly (not
  // deferred): seeds kind 24200 for fresh identities, no-ops for explicit
  // opt-outs. Frames before the listener opens are permanently lost.
  const observerReconciled = useObserverArchiveReconciliation(
    identityQuery.data?.pubkey,
  );
  // useArchiveSync must wait for reconciliation, or listeners could open
  // before kind 24200 is guaranteed present in the subscription.
  useArchiveSync(observerReconciled);
  // The archive batch now persists in Rust, so the agent-metrics invalidation
  // signal arrives as a Tauri event rather than an in-process call.
  useArchiveAgentMetricsBridge();
  // Kind 44200 is relay-persisted (durable) and stays deferred: missed
  // startup frames can be replayed, so there's no ordering constraint.
  const deferredPubkey = startupReady ? identityQuery.data?.pubkey : undefined;
  useAgentMetricArchiveSeed(deferredPubkey);
  const profileQuery = useProfileQuery();
  useRelayAutoHeal();
  usePresenceSubscription();
  useUserStatusSubscription();
  useCommunityEmojiLiveUpdates();
  useMembershipNotifications(identityQuery.data?.pubkey);
  const presenceSession = usePresenceSession(deferredPubkey);
  const selfStatusQuery = useUserStatusQuery(
    deferredPubkey ? [deferredPubkey] : [],
  );
  const setUserStatusMutation = useSetUserStatusMutation(deferredPubkey);
  const { feedProfilesQuery, homeFeedQuery, notificationSettings } =
    useHomeFeedNotifications(identityQuery.data?.pubkey);
  const feedItemState = useFeedItemState(identityQuery.data?.pubkey);
  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data ?? [];
  const refetchHomeFeedFromLiveSignal = React.useEffectEvent(() => {
    void homeFeedQuery.refetch();
  });
  const { refetch: refetchChannels } = channelsQuery;
  const channelsErrorMessage =
    channelsQuery.error instanceof Error
      ? channelsQuery.error.message
      : undefined;
  // People see a plain sentence; the relay's own text goes to the log only.
  const channelsErrorDisplay = channelsErrorMessage
    ? plainRelayErrorMessage(channelsErrorMessage)
    : undefined;
  React.useEffect(() => {
    if (channelsErrorMessage) {
      console.warn("[sidebar] channels request failed:", channelsErrorMessage);
    }
  }, [channelsErrorMessage]);
  useMembershipDenialWatcher(communitiesHook.activeCommunity?.id ?? null);
  const relayConnectionCard = useSidebarRelayConnectionCard(
    channelsErrorMessage,
    communitiesHook.activeCommunity?.relayUrl,
    `${communitiesHook.activeCommunity?.id ?? "none"}-${communitiesHook.reinitKey}`,
  );
  const memberChannels = React.useMemo(
    () => channels.filter((channel) => channel.isMember),
    [channels],
  );
  const sidebarChannels = React.useMemo(
    () => memberChannels.filter((channel) => channel.archivedAt === null),
    [memberChannels],
  );
  const hasRestoredCommunityDestinationRef = React.useRef(false);
  React.useEffect(() => {
    const activeCommunityId = communitiesHook.activeCommunity?.id;
    if (
      hasRestoredCommunityDestinationRef.current ||
      !channelsQuery.isSuccess ||
      channelsQuery.dataUpdatedAt === 0 ||
      !activeCommunityId
    ) {
      return;
    }
    hasRestoredCommunityDestinationRef.current = true;

    // Restoration belongs to an explicit community transition. Cold boot and
    // reconnect remounts must preserve the route the user explicitly opened.
    if (!consumePendingCommunityRestore(activeCommunityId)) {
      return;
    }

    const destination = loadCommunityDestination(activeCommunityId);
    if (!destination || destination.kind === "home") {
      return;
    }

    const channelIsAvailable = sidebarChannels.some(
      (channel) => channel.id === destination.channelId,
    );
    if (!channelIsAvailable) {
      saveCommunityDestination(activeCommunityId, { kind: "home" });
      void goHome({ replace: true });
      return;
    }

    // The normal switch path writes the remembered channel into the hash before
    // the target community mounts, so no intermediate Inbox frame is painted.
    // Older transition callers may still arrive at neutral Home; repair those.
    if (selectedView === "home") {
      void goChannel(destination.channelId, { replace: true });
    }
  }, [
    channelsQuery.dataUpdatedAt,
    channelsQuery.isSuccess,
    communitiesHook.activeCommunity?.id,
    goChannel,
    goHome,
    selectedView,
    sidebarChannels,
  ]);
  const [terminalContextOverride, setTerminalContextOverride] =
    React.useState<TerminalContextOverride | null>(null);
  const { activeChannel, terminalContext } = useTerminalContext({
    channelId: selectedChannelId,
    channels,
    locationSearch: location.search,
    pubkey: identityQuery.data?.pubkey,
    relayUrl: communitiesHook.activeCommunity?.relayUrl,
  });
  const effectiveTerminalContext = terminalContextOverride
    ? {
        ...terminalContext,
        channelId: terminalContextOverride.channelId,
        channelName: terminalContextOverride.channelName,
        threadId: null,
      }
    : terminalContext;
  const managedChannel = React.useMemo(() => {
    const targetChannelId = managedChannelId ?? selectedChannelId;
    return targetChannelId
      ? (channels.find((channel) => channel.id === targetChannelId) ?? null)
      : null;
  }, [channels, managedChannelId, selectedChannelId]);
  const {
    handleChannelNotification,
    handleDmNotification,
    handleThreadReplyDesktopNotification,
  } = useAppShellDesktopNotifications({
    channels,
    enabled: true,
    goChannel,
    goHome,
    notificationSettings: notificationSettings.settings,
    openSearchHit,
    pubkey: identityQuery.data?.pubkey,
  });
  const {
    followedRootIds,
    isFollowing: isFollowingThread,
    followThread,
    unfollowThread,
  } = useThreadFollows(identityQuery.data?.pubkey);
  const {
    markAllChannelsRead: markAllChannelReadMarkers,
    markChannelRead,
    markChannelUnread,
    clearChannelUnreadSource,
    unreadChannelIds,
    topLevelUnreadChannelIds,
    unreadChannelCounts,
    highPriorityUnreadChannelIds,
    unreadChannelNotificationCount,
    getEffectiveTimestamp: getChannelReadAt,
    getOwnTimestamp: getOwnReadAt,
    readStateVersion,
    setContextParentResolver,
    participatedRootIds,
    authoredRootIds,
    mentionedRootIds,
    recordThreadInteraction,
    threadActivityItems,
    mutedRootIds,
    muteThread,
    unmuteThread,
  } = useUnreadChannels(sidebarChannels, activeChannel, {
    pubkey: identityQuery.data?.pubkey,
    relayClient,
    relayUrl: communitiesHook.activeCommunity?.relayUrl,
    currentPubkey: identityQuery.data?.pubkey,
    mutedChannelIds,
    notifyForActiveChannel: notificationSettings.settings.notifyWhileViewing,
    onChannelMessage: handleChannelNotification,
    onDmMessage: handleDmNotification,
    onLiveMention: refetchHomeFeedFromLiveSignal,
    onThreadReplyDesktopNotification: handleThreadReplyDesktopNotification,
    followedRootIds,
  });

  const {
    getThreadReadAt,
    markThreadRead,
    getMessageReadAt,
    getChannelActivityItemReadAt,
    markMessageRead,
    threadActivityFeedItems,
    locallyUnreadFeedItems,
    unreadThreadFeedItems,
    unreadThreadChannelIds,
  } = useChannelActivityProjection({
    channels,
    feed: homeFeedQuery.data?.feed,
    unreadFeedItemIds: feedItemState.unreadSet,
    getChannelReadAt,
    getOwnReadAt,
    markChannelRead,
    readStateVersion,
    threadActivityItems,
    mutedRootIds,
  });
  const markAllChannelsRead = React.useCallback(() => {
    markAllReadSources({
      activeChannelId: activeChannel?.id ?? null,
      channelActivityItems: unreadThreadFeedItems,
      markAllChannelReadMarkers,
      markActiveChannelRead: (channelId, createdAt) =>
        markChannelRead(channelId, new Date(createdAt * 1_000).toISOString()),
      undoUnreadFeedItem: feedItemState.undoUnread,
      unreadFeedItemIds: feedItemState.unreadSet,
    });
  }, [
    activeChannel?.id,
    feedItemState.undoUnread,
    feedItemState.unreadSet,
    markAllChannelReadMarkers,
    markChannelRead,
    unreadThreadFeedItems,
  ]);

  const { homeBadgeCount, homeBadgeCountExcludingHighPriority } =
    useHomeFeedNotificationState(
      homeFeedQuery.data,
      identityQuery.data?.pubkey,
      notificationSettings.settings,
      notificationSettings.setDesktopEnabled,
      true,
      selectedView === "home" && !settingsOpen,
      getChannelReadAt,
      readStateVersion,
      highPriorityUnreadChannelIds,
      feedProfilesQuery.data?.profiles,
      mutedChannelIds,
      feedItemState.unreadSet,
      threadActivityFeedItems,
      getThreadReadAt,
      getMessageReadAt,
      channels,
    );
  const isNotifiedForThread = React.useCallback(
    (rootId: string) =>
      !mutedRootIds.has(rootId) &&
      (followedRootIds.has(rootId) ||
        participatedRootIds.has(rootId) ||
        authoredRootIds.has(rootId) ||
        mentionedRootIds.has(rootId)),
    [
      followedRootIds,
      mutedRootIds,
      participatedRootIds,
      authoredRootIds,
      mentionedRootIds,
    ],
  );

  const handleFollowThread = React.useCallback(
    (rootId: string) => {
      followThread(rootId);
      unmuteThread(rootId);
    },
    [followThread, unmuteThread],
  );

  const handleUnfollowThread = React.useCallback(
    (rootId: string) => {
      unfollowThread(rootId);
      muteThread(rootId);
    },
    [unfollowThread, muteThread],
  );

  const createChannelMutation = useCreateChannelMutation();
  const openDmMutation = useOpenDmMutation();
  const hideDmMutation = useHideDmMutation();
  useDmResurfaceFromMessages({
    pubkey: identityQuery.data?.pubkey,
    relayUrl: communitiesHook.activeCommunity?.relayUrl,
    reopen: openDmMutation.mutateAsync,
  });
  const {
    browseDialogType,
    openBrowseChannels: handleOpenBrowseChannels,
    onBrowseDialogOpenChange: handleBrowseDialogOpenChange,
    getCreateSuccess,
  } = useChannelBrowserDialog(() => void refetchChannels());
  const handleOpenSearch = React.useCallback(() => {
    setSearchFocusRequest((request) => request + 1);
    void refetchChannels();
  }, [refetchChannels]);
  const handleOpenChannelSearch = React.useCallback(() => {
    setScopeSearchFocusRequest((request) => request + 1);
    void refetchChannels();
  }, [refetchChannels]);
  React.useEffect(
    () => listenForChannelSearchRequests(handleOpenChannelSearch),
    [handleOpenChannelSearch],
  );

  const handleBrowseChannelJoin = React.useCallback(
    async (channelId: string) => {
      await joinChannel(channelId);
      await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    },
    [queryClient],
  );

  const handleCreateChannel = React.useCallback(
    async (
      {
        description,
        name,
        visibility,
        ttlSeconds,
      }: {
        name: string;
        description?: string;
        visibility: ChannelVisibility;
        ttlSeconds?: number;
      },
      onCreated?: (channelId: string) => void,
    ) => {
      const createdChannel = await createChannelMutation.mutateAsync({
        name,
        description,
        channelType: "stream",
        visibility,
        ttlSeconds,
      });

      await goChannel(createdChannel.id);
      onCreated?.(createdChannel.id);
    },
    [createChannelMutation, goChannel],
  );
  const handleBrowseChannelCreate = React.useCallback(
    async (input: {
      name: string;
      description?: string;
      visibility: ChannelVisibility;
      ttlSeconds?: number;
    }) => {
      await handleCreateChannel(input, getCreateSuccess() ?? undefined);
    },
    [handleCreateChannel, getCreateSuccess],
  );

  const handleHideDm = React.useCallback(
    async (channelId: string) => {
      try {
        await hideDmMutation.mutateAsync(channelId);
      } catch {
        return;
      }

      if (selectedChannelId === channelId) {
        void goHome();
      }
    },
    [goHome, hideDmMutation, selectedChannelId],
  );
  const handleOpenSettings = React.useCallback(
    (section: SettingsSection = DEFAULT_SETTINGS_SECTION) => {
      setIsChannelManagementOpen(false);
      void goSettings(section);
    },
    [goSettings],
  );
  const handleCloseSettings = React.useCallback(
    () => closeSettings(),
    [closeSettings],
  );
  // Section switches rewrite the settings entry rather than stacking one
  // history entry per section, so back always exits settings in one step.
  const handleSettingsSectionChange = React.useCallback(
    (section: SettingsSection) => {
      void goSettings(section, { replace: true });
    },
    [goSettings],
  );

  const handleOpenSearchResult = React.useCallback(
    (hit: SearchHit, query: string) => {
      void openSearchHit(hit, { query });
    },
    [openSearchHit],
  );
  useAppShellLifecycleEffects({
    desktopBadgeEnabled: true,
    homeBadgeCountExcludingHighPriority,
    topLevelUnreadChannelIds,
    unreadChannelNotificationCount,
  });
  useAppDeepLinks();
  const handleOpenCreateChannel = React.useCallback(
    () => setIsCreateChannelOpen(true),
    [],
  );
  useAppShellKeyboardShortcuts({
    canSearchCurrentChannel:
      selectedView === "channel" && Boolean(activeChannel),
    disabled: settingsOpen,
    onBrowseChannels: handleOpenBrowseChannels,
    onCreateChannel: handleOpenCreateChannel,
    onGoHome: goHome,
    onNewMessage: goNewMessage,
    onSearchCurrentChannel: handleOpenChannelSearch,
    onSearchEverything: handleOpenSearch,
  });
  useSettingsShortcuts({
    onClose: handleCloseSettings,
    onOpenSettings: handleOpenSettings,
    open: settingsOpen,
  });
  useMarkAsReadShortcuts({
    activeChannelId: activeChannel?.id ?? null,
    activeChannelLastMessageAt: activeChannel?.lastMessageAt,
    markAllChannelsRead,
    markChannelRead,
    selectedView,
  });
  return (
    <PreventSleepProvider>
      <AppShellTrayMenu
        channels={channels}
        goChannel={goChannel}
        openCreateChannel={handleOpenCreateChannel}
      />
      <ChannelNavigationProvider channels={channels}>
        <AppShellProvider
          value={{
            navigationHistory: { canGoBack, canGoForward, goBack, goForward },
            markAllChannelsRead,
            markChannelRead,
            markChannelUnread,
            clearChannelUnreadSource,
            openBrowseChannels: handleOpenBrowseChannels,
            openCreateChannel: handleOpenCreateChannel,
            openChannelManagement: (channelId?: string) => {
              setManagedChannelId(
                typeof channelId === "string" ? channelId : null,
              );
              setIsChannelManagementOpen(true);
            },
            getChannelReadAt,
            getThreadReadAt,
            markThreadRead,
            getMessageReadAt,
            getChannelActivityItemReadAt,
            markMessageRead,
            readStateVersion,
            setContextParentResolver,
            followThread: handleFollowThread,
            unfollowThread: handleUnfollowThread,
            isFollowingThread,
            isNotifiedForThread,
            recordThreadInteraction,
            isThreadMuted: (rootId) => mutedRootIds.has(rootId),
            threadActivityItems,
            threadActivityFeedItems,
            locallyUnreadFeedItems,
            unreadThreadFeedItems,
            unreadThreadChannelIds,
            topLevelUnreadChannelIds,
            hasSidebarUnreadProjections: true,
            feedItemState,
            onOpenSettings: handleOpenSettings,
          }}
        >
          <AppShellFrame>
            {hasCommunityRail ? (
              <CommunityRail
                activeCommunityId={communitiesHook.activeCommunity?.id ?? null}
                onAddCommunity={() => openWorkspaceDialog("create")}
                onReorderCommunities={communitiesHook.reorderCommunities}
                onSwitchCommunity={handleSwitchCommunity}
                onUpdateCommunity={communitiesHook.updateCommunity}
                communities={communitiesHook.communities}
              />
            ) : null}
            <SidebarProvider
              className="relative z-10 min-h-0 min-w-0 flex-1 flex-col overflow-visible"
              data-colony-workspace-route={
                !settingsOpen &&
                (location.pathname === "/today" ||
                  location.pathname.startsWith("/today/") ||
                  location.pathname.startsWith("/navigation/") ||
                  selectedView === "channel" ||
                  selectedView === "pins" ||
                  // Commitment and tracking screens under /work still draw
                  // their own header; the work list and a work item do not.
                  (selectedView === "work" &&
                    location.pathname.split("/").filter(Boolean).length > 2))
                  ? "true"
                  : undefined
              }
              data-testid="app-sidebar-layer"
            >
              <AppProfilePanelProvider>
                {settingsOpen ? (
                  <div className="flex min-h-0 flex-1 overflow-hidden">
                    <React.Suspense fallback={null}>
                      <LazySettingsScreen
                        canGoBack={canGoBack}
                        canGoForward={canGoForward}
                        currentPubkey={identityQuery.data?.pubkey}
                        fallbackDisplayName={identityQuery.data?.displayName}
                        isUpdatingDesktopNotifications={
                          notificationSettings.isUpdatingDesktopEnabled
                        }
                        notificationErrorMessage={
                          notificationSettings.errorMessage
                        }
                        notificationPermission={notificationSettings.permission}
                        notificationSettings={notificationSettings.settings}
                        onClose={handleCloseSettings}
                        onGoBack={goBack}
                        onGoForward={goForward}
                        onSectionChange={handleSettingsSectionChange}
                        onSetDesktopNotificationsEnabled={
                          notificationSettings.setDesktopEnabled
                        }
                        onSetHomeBadgeEnabled={
                          notificationSettings.setHomeBadgeEnabled
                        }
                        onSetSlotAlertsEnabled={
                          notificationSettings.setSlotAlertsEnabled
                        }
                        onSetNotifyWhileViewing={
                          notificationSettings.setNotifyWhileViewing
                        }
                        onSetAllSlotAlertsEnabled={
                          notificationSettings.setAllSlotAlertsEnabled
                        }
                        onSetSoundForSlot={notificationSettings.setSoundForSlot}
                        section={settingsSection}
                      />
                    </React.Suspense>
                  </div>
                ) : (
                  <div
                    className="relative flex min-h-0 flex-1 overflow-visible"
                    data-colony-workspace-frame-content
                  >
                    <AppSidebar
                      activeCommunity={communitiesHook.activeCommunity}
                      channels={sidebarChannels}
                      currentPubkey={identityQuery.data?.pubkey}
                      errorMessage={channelsErrorDisplay}
                      fallbackDisplayName={identityQuery.data?.displayName}
                      homeBadgeCount={homeBadgeCount}
                      isPowerActive={location.pathname === "/power"}
                      addCommunityPrefill={addCommunityDialog.prefill}
                      isAddCommunityOpen={addCommunityDialog.open}
                      workspaceDialogMode={workspaceDialogMode}
                      relayConnectionCard={relayConnectionCard}
                      isCreatingChannel={createChannelMutation.isPending}
                      isLoading={channelsQuery.isLoading}
                      isCreateChannelOpen={isCreateChannelOpen}
                      isPresencePending={presenceSession.isPending}
                      onAddCommunity={(community) => {
                        const id = communitiesHook.addCommunity({
                          ...community,
                          pubkey:
                            community.pubkey ?? identityQuery.data?.pubkey,
                        });
                        handleSwitchCommunity(id);
                      }}
                      onAddCommunityOpenChange={
                        handleAddCommunityDialogOpenChange
                      }
                      onNewMessage={goNewMessage}
                      onBackgroundClick={requestFocusedThreadClose}
                      onCreateChannelOpenChange={setIsCreateChannelOpen}
                      onOpenWorkspaceDialog={openWorkspaceDialog}
                      onOpenWorkspaceSettings={openWorkspaceSettings}
                      onSendFeedback={() => setIsSendFeedbackOpen(true)}
                      onSwitchCommunity={handleSwitchCommunity}
                      onRemoveWorkspace={handleRemoveCommunity}
                      onCreateAgent={() => requestOpenCreateAgent()}
                      selfPresenceStatus={presenceSession.currentStatus}
                      communities={communitiesHook.communities}
                      onCreateChannel={handleCreateChannel}
                      onHideDm={handleHideDm}
                      onMarkAllChannelsRead={markAllChannelsRead}
                      onMarkChannelRead={markChannelRead}
                      onMarkChannelUnread={markChannelUnread}
                      onBrowseChannels={handleOpenBrowseChannels}
                      onOpenDm={async ({ pubkeys }) => {
                        const directMessage = await openDmMutation.mutateAsync({
                          pubkeys,
                        });
                        await goChannel(directMessage.id);
                      }}
                      onSelectToday={() => void goToday()}
                      onSelectPower={() => void goPower()}
                      onSelectChannel={handleSidebarChannelSelect}
                      onOpenSearchResult={handleOpenSearchResult}
                      searchChannels={channels}
                      searchFocusRequests={[
                        searchFocusRequest,
                        scopeSearchFocusRequest,
                      ]}
                      onSelectHome={() => void goHome()}
                      onSelectAgents={() => void goAgents()}
                      onSelectTeam={() => void goTeam()}
                      onSelectClients={() => void goClients()}
                      onSelectWork={() => void goWork()}
                      onSelectSettings={handleOpenSettings}
                      onSetPresenceStatus={(status) =>
                        presenceSession.setStatus(status)
                      }
                      onSetUserStatus={setUserStatusMutation.mutate}
                      onClearUserStatus={() =>
                        setUserStatusMutation.mutate({
                          text: "",
                          emoji: "",
                        })
                      }
                      profile={profileQuery.data}
                      showSidebarCollapseButton
                      suppressTodaySelection={location.pathname.startsWith(
                        "/navigation/",
                      )}
                      selfUserStatus={
                        deferredPubkey
                          ? (visibleUserStatus(
                              selfStatusQuery.data?.[
                                deferredPubkey.toLowerCase()
                              ],
                            ) ?? undefined)
                          : undefined
                      }
                      selectedChannelId={selectedChannelId}
                      selectedView={selectedView}
                      unreadChannelIds={unreadChannelIds}
                      {...{ highPriorityUnreadChannelIds }}
                      previewActivityChannelIds={unreadThreadChannelIds}
                      unreadChannelCounts={unreadChannelCounts}
                      mutedChannelIds={mutedChannelIds}
                      onMuteChannel={muteChannel}
                      onUnmuteChannel={unmuteChannel}
                      starredChannelIds={starredChannelIds}
                      onStarChannel={starChannel}
                      onUnstarChannel={unstarChannel}
                    />
                    <TerminalContextOverrideProvider
                      onChange={setTerminalContextOverride}
                    >
                      <AppShellChannelSurface
                        topChrome={
                          isNarrowWindow ? (
                            <MobileWorkspaceHeader
                              activeWorkspace={communitiesHook.activeCommunity}
                              workspaces={communitiesHook.communities}
                              onCopyWorkspaceLink={() => {
                                const workspace =
                                  communitiesHook.activeCommunity;
                                if (workspace) {
                                  copyTextToClipboard(
                                    workspace.relayUrl,
                                    "Workspace link copied.",
                                  );
                                }
                              }}
                              onCreateWorkspace={() =>
                                openWorkspaceDialog("create")
                              }
                              onInvitePeople={() =>
                                handleOpenSettings("community-members")
                              }
                              onJoinWorkspace={() =>
                                openWorkspaceDialog("join")
                              }
                              onOpenWorkspaceSettings={openWorkspaceSettings}
                              onSwitchWorkspace={handleSwitchCommunity}
                            />
                          ) : showAppTopChrome ? (
                            <AppTopChrome
                              canGoBack={canGoBack}
                              canGoForward={canGoForward}
                              hasCommunityRail={hasCommunityRail}
                              onGoBack={goBack}
                              onGoForward={goForward}
                            />
                          ) : null
                        }
                        hasCommunityRail={hasCommunityRail}
                        mainInsetRef={mainInsetRef}
                        terminal={
                          <TerminalBootstrap {...effectiveTerminalContext} />
                        }
                      >
                        <Outlet />
                      </AppShellChannelSurface>
                    </TerminalContextOverrideProvider>
                    <EditCommunityDialog
                      community={workspaceSettingsCommunity}
                      onOpenChange={(open) => {
                        if (!open) setWorkspaceSettingsCommunity(null);
                      }}
                      onSave={communitiesHook.updateCommunity}
                      open={workspaceSettingsCommunity !== null}
                      showIconEditor={
                        workspaceSettingsCommunity?.id ===
                        communitiesHook.activeCommunity?.id
                      }
                    />
                    <RelayConnectionOverlay
                      card={relayConnectionCard}
                      errorMessage={channelsErrorDisplay}
                      hasCommunityRail={hasCommunityRail}
                    />
                  </div>
                )}
                <RequestedAgentCreateDialogs />
                <AgentManagementDialogs />
                <AppShellOverlays
                  activeChannel={managedChannel}
                  browseDialogType={browseDialogType}
                  channels={channels}
                  currentPubkey={identityQuery.data?.pubkey}
                  isChannelManagementOpen={isChannelManagementOpen}
                  isCreatingBrowseChannel={createChannelMutation.isPending}
                  onBrowseChannelJoin={handleBrowseChannelJoin}
                  onBrowseChannelCreate={handleBrowseChannelCreate}
                  onBrowseDialogOpenChange={handleBrowseDialogOpenChange}
                  onChannelManagementOpenChange={(open) => {
                    setIsChannelManagementOpen(open);
                    if (!open) {
                      setManagedChannelId(null);
                    }
                  }}
                  onDeleteActiveChannel={() => {
                    setIsChannelManagementOpen(false);
                    setManagedChannelId(null);
                    void goHome({ replace: true });
                  }}
                  onSelectChannel={(channelId) => {
                    void goChannel(channelId);
                  }}
                  relayUrl={communitiesHook.activeCommunity?.relayUrl}
                />
                <SendFeedbackController
                  onOpenChange={setIsSendFeedbackOpen}
                  open={isSendFeedbackOpen}
                />
                <ProtectedGlobalOverlay />
              </AppProfilePanelProvider>
            </SidebarProvider>
          </AppShellFrame>
        </AppShellProvider>
      </ChannelNavigationProvider>
    </PreventSleepProvider>
  );
}

import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Outlet, useLocation } from "@tanstack/react-router";

import { deriveShellRoute } from "@/app/AppShell.helpers";
import { AppShellProvider } from "@/app/AppShellContext";
import * as BuzzTheme from "@/app/BuzzThemeSurfaces";
import { AppShellOverlays } from "@/app/AppShellOverlays";
import { AppTopChrome } from "@/app/AppTopChrome";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useBackForwardControls } from "@/app/navigation/useBackForwardControls";
import { useLiveHomeFeedActions } from "@/app/useLiveHomeFeedActions";
import { useChannelBrowserDialog } from "@/app/useChannelBrowserDialog";
import { useMarkAsReadShortcuts } from "@/app/useMarkAsReadShortcuts";
import { useSettingsShortcuts } from "@/app/useSettingsShortcuts";
import { useAppShellDesktopNotifications } from "@/app/useAppShellDesktopNotifications";
import { useThreadActivityFeedItems } from "@/app/useThreadActivityFeedItems";
import { useTauriWindowDrag } from "@/app/useTauriWindowDrag";
import { useWebviewZoomShortcuts } from "@/app/useWebviewZoomShortcuts";
import {
  channelsQueryKey,
  useChannelsQuery,
  useCreateChannelMutation,
  useHideDmMutation,
  useOpenDmMutation,
} from "@/features/channels/hooks";
import { useUnreadChannels } from "@/features/channels/useUnreadChannels";
import { msgContextKey } from "@/features/channels/readState/readStateFormat";
import { useMembershipNotifications } from "@/features/channels/useMembershipNotifications";
import { useFeedItemState } from "@/features/home/useFeedItemState";
import { useThreadFollows } from "@/features/messages/lib/useThreadFollows";
import {
  useHomeFeedNotifications,
  useHomeFeedNotificationState,
} from "@/features/notifications/hooks";
import { setDesktopAppBadge } from "@/features/notifications/lib/desktop";
import { PreventSleepProvider } from "@/features/agents/usePreventSleep";
import { requestOpenCreateAgent } from "@/features/agents/openCreateAgentEvent";
import { useAgentsDataRefresh } from "@/features/agents/lib/useAgentsDataRefresh";
import { useAutoRestartPolicy } from "@/features/agents/lib/useAutoRestartPolicy";
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
  useUserStatusSubscription,
} from "@/features/user-status/hooks";
import { useCommunityEmojiLiveUpdates } from "@/features/custom-emoji/hooks";
import { useArchiveSync } from "@/features/local-archive/archiveSyncManager";
import { useObserverArchiveReconciliation } from "@/features/local-archive/useObserverArchiveSeed";
import { useAgentMetricArchiveSeed } from "@/features/local-archive/useAgentMetricArchiveSeed";
import { useProfileQuery } from "@/features/profile/hooks";
import { SendFeedbackController } from "@/features/settings/ui/SendFeedbackController";
import {
  DEFAULT_SETTINGS_SECTION,
  type SettingsSection,
  isSettingsSection,
} from "@/features/settings/ui/SettingsPanels";
import { HuddleBar, HuddleProvider } from "@/features/huddle";
import { useDueReminderBadgeCount } from "@/features/reminders/hooks";
import { RemindMeLaterProvider } from "@/features/reminders/ui/RemindMeLaterProvider";
import { useReminderNotifications } from "@/features/reminders/useReminderNotifications";
import { AppSidebar } from "@/features/sidebar/ui/AppSidebar";
import { CommunityRail } from "@/features/sidebar/ui/CommunityRail";
import { useChannelMutes } from "@/features/sidebar/lib/useChannelMutes";
import { useChannelStars } from "@/features/sidebar/lib/useChannelStars";
import { useCommunities } from "@/features/communities/useCommunities";
import { useAddCommunityDialogState } from "@/features/communities/addCommunityPrefill";
import { useApplyTemplate } from "@/features/channel-templates/useApplyTemplate";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useRelayAutoHeal } from "@/shared/api/useRelayAutoHeal";
import { useDeferredStartup } from "@/shared/hooks/useDeferredStartup";
import { useWebviewScrollBoundaryLock } from "@/shared/hooks/useWebviewScrollBoundaryLock";
import { joinChannel } from "@/shared/api/tauri";
import type { ChannelVisibility, SearchHit } from "@/shared/api/types";
import { ChannelNavigationProvider } from "@/shared/context/ChannelNavigationContext";
import { MainInsetProvider } from "@/shared/layout/MainInsetContext";
import { chromeCssVarDefaults } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { useMessageDeepLinks } from "@/shared/useMessageDeepLinks";
import { SidebarInset, SidebarProvider } from "@/shared/ui/sidebar";
import { RelayConnectionOverlay } from "@/app/RelayConnectionOverlay";
import { useSidebarRelayConnectionCard } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";

const LazySettingsScreen = React.lazy(async () => {
  const module = await import("@/features/settings/ui/SettingsScreen");
  return { default: module.SettingsScreen };
});

export function AppShell() {
  useWebviewZoomShortcuts();
  useTauriWindowDrag();
  useWebviewScrollBoundaryLock();

  const communitiesHook = useCommunities();
  const hasCommunityRail = communitiesHook.communities.length > 1;
  const addCommunityDialog = useAddCommunityDialogState();
  const [isChannelManagementOpen, setIsChannelManagementOpen] =
    React.useState(false);
  const [managedChannelId, setManagedChannelId] = React.useState<string | null>(
    null,
  );
  const [searchFocusRequest, setSearchFocusRequest] = React.useState(0);
  const [isCreateChannelOpen, setIsCreateChannelOpen] = React.useState(false);
  const [isSendFeedbackOpen, setIsSendFeedbackOpen] = React.useState(false);
  const [isHuddleDrawerOpen, setIsHuddleDrawerOpen] = React.useState(false);
  const mainInsetRef = React.useRef<HTMLElement>(null);
  const location = useLocation();
  const queryClient = useQueryClient();
  const {
    goAgents,
    goChannel,
    goHome,
    goNewMessage,
    goProjects,
    goPulse,
    goSettings,
    goWorkflows,
    closeSettings,
    openSearchHit,
  } = useAppNavigation();
  const { canGoBack, canGoForward, goBack, goForward } =
    useBackForwardControls();
  // Navigate home before switching communities so the outgoing channel URL is
  // cleared. Without this, ChannelScreen's read effect continues firing
  // markChannelRead({ topLevelOnly: true }) for the previous community's
  // channel, advancing its NIP-RS markers and causing the rail badge to vanish
  // on the next 30s poll (A→B→A→B disappearance bug).
  // Guard: skip goHome() when re-selecting the already-active community so
  // the current channel is not unexpectedly cleared.
  const handleSwitchCommunity = React.useCallback(
    (id: string) => {
      if (id !== communitiesHook.activeCommunity?.id) {
        void goHome();
      }
      communitiesHook.switchCommunity(id);
    },
    [
      goHome,
      communitiesHook.activeCommunity?.id,
      communitiesHook.switchCommunity,
    ],
  );
  const { selectedChannelId, selectedView } = React.useMemo(
    () => deriveShellRoute(location.pathname),
    [location.pathname],
  );
  // Settings lives in history so back returns to the previous app entry.
  const settingsOpen = location.pathname === "/settings";
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
  );
  const { starredChannelIds, starChannel, unstarChannel } = useChannelStars(
    identityQuery.data?.pubkey,
  );
  usePersonaSync(identityQuery.data?.pubkey);
  useAgentsDataRefresh();
  // Chunk F: auto-restart drifted idle agents (per-agent opt-out, default ON).
  useAutoRestartPolicy();
  // Owner-global observer ingestion: receives + decrypts agent observer
  // frames and keeps derived active-turn liveness in sync app-wide, so no
  // individual screen/panel has to mount its own bridge for ingestion.
  // Intentionally mounted without a `startupReady`/identity guard: before
  // `currentPubkey` resolves the hook ingests managed agents only, and
  // relay-owned agents join automatically once identity arrives. Adding a
  // guard here would drop managed-agent coverage during startup.
  useAgentObserverIngestion();
  // Kind 24200 is relay-ephemeral, so reconciliation runs eagerly (not
  // deferred) and unconditionally repairs the DB subscription on internal
  // builds — otherwise frames emitted before the listener opens are lost.
  const observerReconciled = useObserverArchiveReconciliation(
    identityQuery.data?.pubkey,
  );
  // useArchiveSync must wait for reconciliation, or listeners could open
  // before kind 24200 is guaranteed present in the subscription.
  useArchiveSync(observerReconciled);
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
  useReminderNotifications(
    identityQuery.data?.pubkey,
    notificationSettings.settings,
    channels,
  );
  const refetchHomeFeedFromLiveSignal = React.useEffectEvent(() => {
    void homeFeedQuery.refetch();
  });
  useLiveHomeFeedActions(
    identityQuery.data?.pubkey,
    refetchHomeFeedFromLiveSignal,
  );
  const { refetch: refetchChannels } = channelsQuery;
  const channelsErrorMessage =
    channelsQuery.error instanceof Error
      ? channelsQuery.error.message
      : undefined;
  const relayConnectionCard = useSidebarRelayConnectionCard(
    channelsErrorMessage,
    communitiesHook.activeCommunity?.relayUrl,
  );
  const memberChannels = React.useMemo(
    () => channels.filter((channel) => channel.isMember),
    [channels],
  );
  const sidebarChannels = React.useMemo(
    () => memberChannels.filter((channel) => channel.archivedAt === null),
    [memberChannels],
  );
  const activeChannel = React.useMemo(
    () =>
      selectedChannelId
        ? (channels.find((channel) => channel.id === selectedChannelId) ?? null)
        : null,
    [channels, selectedChannelId],
  );
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
    markAllChannelsRead,
    markChannelRead,
    markChannelUnread,
    unreadChannelIds,
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

  const getThreadReadAt = React.useCallback(
    (rootId: string, channelId?: string | null) => {
      const threadReadAt = getOwnReadAt(`thread:${rootId}`);
      if (!channelId) {
        return threadReadAt;
      }

      const channelReadAt = getChannelReadAt(channelId);
      if (threadReadAt === null) {
        return channelReadAt;
      }
      if (channelReadAt === null) {
        return threadReadAt;
      }
      return Math.max(threadReadAt, channelReadAt);
    },
    [getChannelReadAt, getOwnReadAt],
  );

  const markThreadRead = React.useCallback(
    (rootId: string, timestamp: number) => {
      markChannelRead(
        `thread:${rootId}`,
        new Date(timestamp * 1_000).toISOString(),
      );
    },
    [markChannelRead],
  );

  // Per-message read frontier (LP4 v3): effective(msg:<id>) folds through the
  // channel, so a channel-read clears messages older than the top-level frontier.
  const getMessageReadAt = React.useCallback(
    (messageId: string) => getChannelReadAt(msgContextKey(messageId)),
    [getChannelReadAt],
  );
  const markMessageRead = React.useCallback(
    (messageId: string, timestamp: number) =>
      markChannelRead(
        msgContextKey(messageId),
        new Date(timestamp * 1_000).toISOString(),
      ),
    [markChannelRead],
  );
  const threadActivityFeedItems = useThreadActivityFeedItems(
    threadActivityItems,
    mutedRootIds,
    channels,
  );

  // Badge count consumes the shared NIP-RS read-state from useUnreadChannels.
  const { homeBadgeCount, homeBadgeCountExcludingHighPriority } =
    useHomeFeedNotificationState(
      homeFeedQuery.data,
      identityQuery.data?.pubkey,
      notificationSettings.settings,
      notificationSettings.setDesktopEnabled,
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

  const dueReminderBadge = useDueReminderBadgeCount(
    identityQuery.data?.pubkey,
    notificationSettings.settings.homeBadgeEnabled,
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

  const createChannelMutation = useCreateChannelMutation(),
    createForumMutation = useCreateChannelMutation();
  const { applyCanvas, applyAgents } = useApplyTemplate();
  const openDmMutation = useOpenDmMutation();
  const hideDmMutation = useHideDmMutation();
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
        templateId,
      }: {
        name: string;
        description?: string;
        visibility: ChannelVisibility;
        ttlSeconds?: number;
        templateId?: string;
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

      await applyCanvas(templateId, createdChannel.id, name);
      await goChannel(createdChannel.id);
      onCreated?.(createdChannel.id);
      void applyAgents(templateId, createdChannel.id);
    },
    [applyAgents, applyCanvas, createChannelMutation, goChannel],
  );

  const handleCreateForum = React.useCallback(
    async ({
      description,
      name,
      visibility,
      ttlSeconds,
      templateId,
    }: {
      name: string;
      description?: string;
      visibility: ChannelVisibility;
      ttlSeconds?: number;
      templateId?: string;
    }) => {
      const createdForum = await createForumMutation.mutateAsync({
        name,
        description,
        channelType: "forum",
        visibility,
        ttlSeconds,
      });

      await applyCanvas(templateId, createdForum.id, name);
      await goChannel(createdForum.id);
      void applyAgents(templateId, createdForum.id);
    },
    [applyAgents, applyCanvas, createForumMutation, goChannel],
  );

  // The channel browser can create either a stream or a forum depending on
  // which section opened it. Route to the matching handler.
  const handleBrowseChannelCreate = React.useCallback(
    async (input: {
      name: string;
      description?: string;
      visibility: ChannelVisibility;
      ttlSeconds?: number;
      templateId?: string;
    }) => {
      if (browseDialogType === "forum") {
        await handleCreateForum(input);
      } else {
        await handleCreateChannel(input, getCreateSuccess() ?? undefined);
      }
    },
    [
      browseDialogType,
      handleCreateChannel,
      handleCreateForum,
      getCreateSuccess,
    ],
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
    (hit: SearchHit) => {
      void openSearchHit(hit);
    },
    [openSearchHit],
  );

  // Prevent webview file:/// navigation on file drop outside the composer.
  // Scoped to file drags only (text drag-and-drop into inputs still works).
  // Composer's onDrop fires first (React synthetic before window bubble).
  React.useEffect(() => {
    function preventNavigation(e: DragEvent) {
      if (e.dataTransfer?.types.includes("Files")) {
        e.preventDefault();
      }
    }
    window.addEventListener("dragover", preventNavigation);
    window.addEventListener("drop", preventNavigation);
    return () => {
      window.removeEventListener("dragover", preventNavigation);
      window.removeEventListener("drop", preventNavigation);
    };
  }, []);

  React.useEffect(() => {
    let isCancelled = false;

    const startPreconnect = () => {
      if (isCancelled) {
        return;
      }

      void relayClient.preconnect().catch((error) => {
        if (!isCancelled) {
          console.error("Failed to preconnect to relay", error);
        }
      });
    };

    if ("requestIdleCallback" in window) {
      const idleId = window.requestIdleCallback(startPreconnect, {
        timeout: 1_500,
      });
      return () => {
        isCancelled = true;
        window.cancelIdleCallback(idleId);
      };
    }

    const timeoutId = globalThis.setTimeout(startPreconnect, 250);
    return () => {
      isCancelled = true;
      globalThis.clearTimeout(timeoutId);
    };
  }, []);

  React.useEffect(() => {
    const count =
      unreadChannelNotificationCount + homeBadgeCountExcludingHighPriority;
    void setDesktopAppBadge(
      count
        ? { kind: "count", count }
        : { kind: unreadChannelIds.size ? "dot" : "none" },
    );
  }, [
    homeBadgeCountExcludingHighPriority,
    unreadChannelIds,
    unreadChannelNotificationCount,
  ]);

  // Dispatch `buzz://message` deep links into the router.
  useMessageDeepLinks();

  const handleOpenNewDm = React.useCallback(
    () => void goNewMessage(),
    [goNewMessage],
  );
  const handleOpenCreateChannel = React.useCallback(
    () => setIsCreateChannelOpen(true),
    [],
  );
  React.useLayoutEffect(() => {
    if (settingsOpen) {
      return;
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (!hasPrimaryShortcutModifier(event) || event.altKey || event.repeat) {
        return;
      }

      // A focused surface may claim the shortcut first — e.g. the composer
      // consumes ⌘K to open the link editor when text is selected. Its
      // element-level handler runs before this window-level bubble listener
      // and calls `preventDefault()`; respect that instead of also opening
      // the global dialog.
      if (event.defaultPrevented) {
        return;
      }

      const key = event.key.toLowerCase();
      if (key === "k" && !event.shiftKey) {
        event.preventDefault();
        handleOpenSearch();
        return;
      }

      if (key === "k" && event.shiftKey) {
        event.preventDefault();
        handleOpenNewDm();
        return;
      }

      if (key === "n" && event.shiftKey) {
        event.preventDefault();
        handleOpenCreateChannel();
        return;
      }

      if (key === "o" && event.shiftKey) {
        event.preventDefault();
        handleOpenBrowseChannels();
        return;
      }

      if (key === "a" && event.shiftKey) {
        event.preventDefault();
        void goHome();
        return;
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [
    handleOpenBrowseChannels,
    handleOpenNewDm,
    handleOpenCreateChannel,
    handleOpenSearch,
    goHome,
    settingsOpen,
  ]);
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
      <ChannelNavigationProvider channels={channels}>
        <AppShellProvider
          value={{
            markAllChannelsRead,
            markChannelRead,
            markChannelUnread,
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
            markMessageRead,
            readStateVersion,
            setContextParentResolver,
            followThread: handleFollowThread,
            unfollowThread: handleUnfollowThread,
            isFollowingThread,
            isNotifiedForThread,
            isThreadMuted: (rootId) => mutedRootIds.has(rootId),
            threadActivityItems,
            threadActivityFeedItems,
            feedItemState,
            onOpenSettings: handleOpenSettings,
          }}
        >
          <HuddleProvider>
            <RemindMeLaterProvider pubkey={identityQuery.data?.pubkey}>
              <div
                className="buzz-huddle-shell relative h-dvh overflow-hidden overscroll-none"
                data-huddle-open={isHuddleDrawerOpen}
              >
                <div
                  className={cn(
                    "buzz-huddle-app-surface z-10 flex min-h-0 flex-row overflow-hidden bg-background",
                    isHuddleDrawerOpen && "buzz-huddle-app-surface-open",
                  )}
                >
                  <BuzzTheme.GradientLayer />
                  {hasCommunityRail ? (
                    <CommunityRail
                      activeCommunityId={
                        communitiesHook.activeCommunity?.id ?? null
                      }
                      onAddCommunity={addCommunityDialog.openDialog}
                      onRemoveCommunity={communitiesHook.removeCommunity}
                      onSwitchCommunity={handleSwitchCommunity}
                      onUpdateCommunity={communitiesHook.updateCommunity}
                      communities={communitiesHook.communities}
                    />
                  ) : null}
                  <SidebarProvider className="min-h-0 flex-1 flex-col overflow-hidden">
                    {!settingsOpen ? (
                      <AppTopChrome
                        canGoBack={canGoBack}
                        canGoForward={canGoForward}
                        hasCommunityRail={hasCommunityRail}
                        onGoBack={goBack}
                        onGoForward={goForward}
                      />
                    ) : null}
                    {settingsOpen ? (
                      <div className="flex min-h-0 flex-1 overflow-hidden">
                        <React.Suspense fallback={null}>
                          <LazySettingsScreen
                            currentPubkey={identityQuery.data?.pubkey}
                            fallbackDisplayName={
                              identityQuery.data?.displayName
                            }
                            isUpdatingDesktopNotifications={
                              notificationSettings.isUpdatingDesktopEnabled
                            }
                            notificationErrorMessage={
                              notificationSettings.errorMessage
                            }
                            notificationPermission={
                              notificationSettings.permission
                            }
                            notificationSettings={notificationSettings.settings}
                            onClose={handleCloseSettings}
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
                            onSetSoundForSlot={
                              notificationSettings.setSoundForSlot
                            }
                            section={settingsSection}
                          />
                        </React.Suspense>
                      </div>
                    ) : (
                      <div className="flex min-h-0 flex-1 overflow-hidden">
                        <AppSidebar
                          activeCommunity={communitiesHook.activeCommunity}
                          channels={sidebarChannels}
                          currentPubkey={identityQuery.data?.pubkey}
                          errorMessage={channelsErrorMessage}
                          fallbackDisplayName={identityQuery.data?.displayName}
                          homeBadgeCount={homeBadgeCount + dueReminderBadge}
                          addCommunityPrefill={addCommunityDialog.prefill}
                          isAddCommunityOpen={addCommunityDialog.open}
                          relayConnectionCard={relayConnectionCard}
                          isCreatingChannel={createChannelMutation.isPending}
                          isCreatingForum={createForumMutation.isPending}
                          isLoading={channelsQuery.isLoading}
                          isCreateChannelOpen={isCreateChannelOpen}
                          isPresencePending={presenceSession.isPending}
                          onAddCommunity={(community) => {
                            const id = communitiesHook.addCommunity(community);
                            handleSwitchCommunity(id);
                          }}
                          onAddCommunityOpenChange={
                            addCommunityDialog.onOpenChange
                          }
                          onNewMessage={handleOpenNewDm}
                          onCreateChannelOpenChange={setIsCreateChannelOpen}
                          onOpenAddCommunity={addCommunityDialog.openDialog}
                          onSendFeedback={() => setIsSendFeedbackOpen(true)}
                          onUpdateCommunity={communitiesHook.updateCommunity}
                          onRemoveCommunity={communitiesHook.removeCommunity}
                          onSwitchCommunity={handleSwitchCommunity}
                          onCreateAgent={() => requestOpenCreateAgent()}
                          selfPresenceStatus={presenceSession.currentStatus}
                          communities={communitiesHook.communities}
                          onCreateChannel={handleCreateChannel}
                          onCreateForum={handleCreateForum}
                          onHideDm={handleHideDm}
                          onMarkAllChannelsRead={markAllChannelsRead}
                          onMarkChannelRead={markChannelRead}
                          onMarkChannelUnread={markChannelUnread}
                          onBrowseChannels={handleOpenBrowseChannels}
                          onOpenDm={async ({ pubkeys }) => {
                            const directMessage =
                              await openDmMutation.mutateAsync({
                                pubkeys,
                              });
                            await goChannel(directMessage.id);
                          }}
                          onSelectAgents={() => void goAgents()}
                          onSelectChannel={(channelId) =>
                            void goChannel(channelId)
                          }
                          onOpenSearchResult={handleOpenSearchResult}
                          searchChannels={channels}
                          searchFocusRequest={searchFocusRequest}
                          onSelectHome={() => void goHome()}
                          onSelectProjects={() => void goProjects()}
                          onSelectPulse={() => void goPulse()}
                          onSelectSettings={handleOpenSettings}
                          onSelectWorkflows={() => void goWorkflows()}
                          onSetPresenceStatus={(status) =>
                            presenceSession.setStatus(status)
                          }
                          onSetUserStatus={(text, emoji) =>
                            setUserStatusMutation.mutate({ text, emoji })
                          }
                          onClearUserStatus={() =>
                            setUserStatusMutation.mutate({
                              text: "",
                              emoji: "",
                            })
                          }
                          profile={profileQuery.data}
                          selfUserStatus={
                            deferredPubkey
                              ? (selfStatusQuery.data?.[
                                  deferredPubkey.toLowerCase()
                                ] ?? undefined)
                              : undefined
                          }
                          selectedChannelId={selectedChannelId}
                          selectedView={selectedView}
                          unreadChannelIds={unreadChannelIds}
                          unreadChannelCounts={unreadChannelCounts}
                          mutedChannelIds={mutedChannelIds}
                          onMuteChannel={muteChannel}
                          onUnmuteChannel={unmuteChannel}
                          starredChannelIds={starredChannelIds}
                          onStarChannel={starChannel}
                          onUnstarChannel={unstarChannel}
                        />
                        <MainInsetProvider mainInsetRef={mainInsetRef}>
                          <SidebarInset
                            ref={mainInsetRef}
                            className="isolate min-h-0 min-w-0 overflow-hidden bg-sidebar"
                            data-buzz-glass-inset
                            data-buzz-shadow-viewport
                            style={chromeCssVarDefaults as React.CSSProperties}
                          >
                            <BuzzTheme.ContentSurface>
                              <Outlet />
                            </BuzzTheme.ContentSurface>
                          </SidebarInset>
                        </MainInsetProvider>
                        <RelayConnectionOverlay
                          card={relayConnectionCard}
                          errorMessage={channelsErrorMessage}
                          hasCommunityRail={hasCommunityRail}
                          isHuddleDrawerOpen={isHuddleDrawerOpen}
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
                      isCreatingBrowseChannel={
                        createChannelMutation.isPending ||
                        createForumMutation.isPending
                      }
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
                    />
                    <SendFeedbackController
                      onOpenChange={setIsSendFeedbackOpen}
                      open={isSendFeedbackOpen}
                    />
                  </SidebarProvider>
                </div>

                <div className="absolute inset-x-0 bottom-0 z-0 h-(--buzz-huddle-drawer-height)">
                  <HuddleBar
                    className="h-full"
                    onOpenThread={(channelId, messageId) => {
                      void goChannel(channelId, {
                        messageId,
                        threadRootId: messageId,
                      });
                    }}
                    onVisibilityChange={setIsHuddleDrawerOpen}
                  />
                </div>
              </div>
            </RemindMeLaterProvider>
          </HuddleProvider>
        </AppShellProvider>
      </ChannelNavigationProvider>
    </PreventSleepProvider>
  );
}

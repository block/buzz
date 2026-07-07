import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Outlet, useLocation } from "@tanstack/react-router";

import { deriveShellRoute } from "@/app/AppShell.helpers";
import { AppShellProvider } from "@/app/AppShellContext";
import {
  AppShellOverlays,
  type BrowseDialogType,
} from "@/app/AppShellOverlays";
import { AppTopChrome } from "@/app/AppTopChrome";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useBackForwardControls } from "@/app/navigation/useBackForwardControls";
import { useLiveHomeFeedActions } from "@/app/useLiveHomeFeedActions";
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
import { usePersonaSync } from "@/features/agents/lib/usePersonaSync";
import { useAgentObserverIngestion } from "@/features/agents/useAgentObserverIngestion";
import {
  usePresenceSession,
  usePresenceSubscription,
} from "@/features/presence/hooks";
import {
  useSetUserStatusMutation,
  useUserStatusQuery,
  useUserStatusSubscription,
} from "@/features/user-status/hooks";
import { useWorkspaceEmojiLiveUpdates } from "@/features/custom-emoji/hooks";
import { useArchiveSync } from "@/features/local-archive/archiveSyncManager";
import { useObserverArchiveSeed } from "@/features/local-archive/useObserverArchiveSeed";
import { useAgentMetricArchiveSeed } from "@/features/local-archive/useAgentMetricArchiveSeed";
import { useProfileQuery } from "@/features/profile/hooks";
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
import { WorkspaceRail } from "@/features/sidebar/ui/WorkspaceRail";
import { useChannelMutes } from "@/features/sidebar/lib/useChannelMutes";
import { useChannelStars } from "@/features/sidebar/lib/useChannelStars";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { useApplyTemplate } from "@/features/channel-templates/useApplyTemplate";
import { relayClient } from "@/shared/api/relayClient";
import { useFeatureEnabled } from "@/shared/features";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useRelayAutoHeal } from "@/shared/api/useRelayAutoHeal";
import { useDeferredStartup } from "@/shared/hooks/useDeferredStartup";
import { useWebviewScrollBoundaryLock } from "@/shared/hooks/useWebviewScrollBoundaryLock";
import { joinChannel } from "@/shared/api/tauri";
import type { SearchHit } from "@/shared/api/types";
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

  const workspacesHook = useWorkspaces();
  const workspaceRailEnabled = useFeatureEnabled("workspaceRail");
  const [isAddWorkspaceOpen, setIsAddWorkspaceOpen] = React.useState(false);
  const [isChannelManagementOpen, setIsChannelManagementOpen] =
    React.useState(false);
  const [managedChannelId, setManagedChannelId] = React.useState<string | null>(
    null,
  );
  const [searchFocusRequest, setSearchFocusRequest] = React.useState(0);
  const [browseDialogType, setBrowseDialogType] =
    React.useState<BrowseDialogType>(null);
  const [isNewDmOpen, setIsNewDmOpen] = React.useState(false);
  const [isCreateChannelOpen, setIsCreateChannelOpen] = React.useState(false);
  const [isHuddleDrawerOpen, setIsHuddleDrawerOpen] = React.useState(false);
  const mainInsetRef = React.useRef<HTMLElement>(null);
  const location = useLocation();
  const queryClient = useQueryClient();
  const {
    goAgents,
    goChannel,
    goHome,
    goProjects,
    goPulse,
    goSettings,
    goWorkflows,
    closeSettings,
    openSearchHit,
  } = useAppNavigation();
  const { canGoBack, canGoForward, goBack, goForward } =
    useBackForwardControls();
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
  // Owner-global observer ingestion: receives + decrypts agent observer
  // frames and keeps derived active-turn liveness in sync app-wide, so no
  // individual screen/panel has to mount its own bridge for ingestion.
  // Intentionally mounted without a `startupReady`/identity guard: before
  // `currentPubkey` resolves the hook ingests managed agents only, and
  // relay-owned agents join automatically once identity arrives. Adding a
  // guard here would drop managed-agent coverage during startup.
  useAgentObserverIngestion();
  useArchiveSync();
  useObserverArchiveSeed(identityQuery.data?.pubkey);
  useAgentMetricArchiveSeed(identityQuery.data?.pubkey);
  const profileQuery = useProfileQuery();
  const deferredPubkey = startupReady ? identityQuery.data?.pubkey : undefined;
  useRelayAutoHeal();
  usePresenceSubscription();
  useUserStatusSubscription();
  useWorkspaceEmojiLiveUpdates();
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
    workspacesHook.activeWorkspace?.relayUrl,
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

  const createChannelMutation = useCreateChannelMutation();
  const createForumMutation = useCreateChannelMutation();
  const { applyCanvas, applyAgents } = useApplyTemplate();

  const openDmMutation = useOpenDmMutation();
  const hideDmMutation = useHideDmMutation();
  const handleOpenBrowseChannels = React.useCallback(() => {
    setBrowseDialogType("stream");
    void refetchChannels();
  }, [refetchChannels]);
  const handleOpenSearch = React.useCallback(() => {
    setSearchFocusRequest((request) => request + 1);
    void refetchChannels();
  }, [refetchChannels]);

  const handleBrowseDialogOpenChange = React.useCallback((open: boolean) => {
    if (!open) {
      setBrowseDialogType(null);
    }
  }, []);

  const handleBrowseChannelJoin = React.useCallback(
    async (channelId: string) => {
      await joinChannel(channelId);
      await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    },
    [queryClient],
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

  const handleOpenNewDm = React.useCallback(() => setIsNewDmOpen(true), []);
  const handleOpenCreateChannel = React.useCallback(
    () => setIsCreateChannelOpen(true),
    [],
  );
  React.useLayoutEffect(() => {
    if (settingsOpen) {
      return;
    }

    function handleKeyDown(event: KeyboardEvent) {
      if (!hasPrimaryShortcutModifier(event) || event.altKey) {
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
                  {workspaceRailEnabled ? (
                    <WorkspaceRail
                      activeWorkspaceId={
                        workspacesHook.activeWorkspace?.id ?? null
                      }
                      onAddWorkspace={() => setIsAddWorkspaceOpen(true)}
                      onRemoveWorkspace={workspacesHook.removeWorkspace}
                      onSwitchWorkspace={workspacesHook.switchWorkspace}
                      onUpdateWorkspace={workspacesHook.updateWorkspace}
                      workspaces={workspacesHook.workspaces}
                    />
                  ) : null}
                  <SidebarProvider className="min-h-0 flex-1 flex-col overflow-hidden">
                    {!settingsOpen ? (
                      <AppTopChrome
                        canGoBack={canGoBack}
                        canGoForward={canGoForward}
                        hasWorkspaceRail={
                          workspaceRailEnabled &&
                          workspacesHook.workspaces.length > 1
                        }
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
                          activeWorkspace={workspacesHook.activeWorkspace}
                          channels={sidebarChannels}
                          currentPubkey={identityQuery.data?.pubkey}
                          errorMessage={channelsErrorMessage}
                          fallbackDisplayName={identityQuery.data?.displayName}
                          homeBadgeCount={homeBadgeCount + dueReminderBadge}
                          isAddWorkspaceOpen={isAddWorkspaceOpen}
                          relayConnectionCard={relayConnectionCard}
                          isCreatingChannel={createChannelMutation.isPending}
                          isCreatingForum={createForumMutation.isPending}
                          isLoading={channelsQuery.isLoading}
                          isOpeningDm={openDmMutation.isPending}
                          isNewDmOpen={isNewDmOpen}
                          isCreateChannelOpen={isCreateChannelOpen}
                          isPresencePending={presenceSession.isPending}
                          onAddWorkspace={(workspace) => {
                            const id = workspacesHook.addWorkspace(workspace);
                            workspacesHook.switchWorkspace(id);
                          }}
                          onAddWorkspaceOpenChange={setIsAddWorkspaceOpen}
                          onNewDmOpenChange={setIsNewDmOpen}
                          onCreateChannelOpenChange={setIsCreateChannelOpen}
                          onOpenAddWorkspace={() => setIsAddWorkspaceOpen(true)}
                          onUpdateWorkspace={workspacesHook.updateWorkspace}
                          onRemoveWorkspace={workspacesHook.removeWorkspace}
                          onSwitchWorkspace={workspacesHook.switchWorkspace}
                          onCreateAgent={() =>
                            void goAgents().then(requestOpenCreateAgent)
                          }
                          selfPresenceStatus={presenceSession.currentStatus}
                          workspaces={workspacesHook.workspaces}
                          onCreateChannel={async ({
                            description,
                            name,
                            visibility,
                            ttlSeconds,
                            templateId,
                          }) => {
                            const createdChannel =
                              await createChannelMutation.mutateAsync({
                                name,
                                description,
                                channelType: "stream",
                                visibility,
                                ttlSeconds,
                              });

                            await applyCanvas(
                              templateId,
                              createdChannel.id,
                              name,
                            );
                            await goChannel(createdChannel.id);
                            void applyAgents(templateId, createdChannel.id);
                          }}
                          onCreateForum={async ({
                            description,
                            name,
                            visibility,
                            ttlSeconds,
                            templateId,
                          }) => {
                            const createdForum =
                              await createForumMutation.mutateAsync({
                                name,
                                description,
                                channelType: "forum",
                                visibility,
                                ttlSeconds,
                              });

                            await applyCanvas(
                              templateId,
                              createdForum.id,
                              name,
                            );
                            await goChannel(createdForum.id);
                            void applyAgents(templateId, createdForum.id);
                          }}
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
                            style={chromeCssVarDefaults as React.CSSProperties}
                          >
                            <div className="relative z-10 mb-2 ml-px mr-2 mt-px flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl bg-background shadow-[-1px_-1px_0_0_hsl(var(--sidebar-border)/0.45)]">
                              <Outlet />
                            </div>
                          </SidebarInset>
                        </MainInsetProvider>
                        <RelayConnectionOverlay
                          card={relayConnectionCard}
                          errorMessage={channelsErrorMessage}
                          hasWorkspaceRail={
                            workspaceRailEnabled &&
                            workspacesHook.workspaces.length > 1
                          }
                          isHuddleDrawerOpen={isHuddleDrawerOpen}
                        />
                      </div>
                    )}
                    <AppShellOverlays
                      activeChannel={managedChannel}
                      browseDialogType={browseDialogType}
                      channels={channels}
                      currentPubkey={identityQuery.data?.pubkey}
                      isChannelManagementOpen={isChannelManagementOpen}
                      onBrowseChannelJoin={handleBrowseChannelJoin}
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

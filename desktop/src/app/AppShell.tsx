import * as React from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useQueryClient } from "@tanstack/react-query";
import { Outlet, useLocation } from "@tanstack/react-router";

import { AppShellProvider } from "@/app/AppShellContext";
import {
  AppShellOverlays,
  type BrowseDialogType,
} from "@/app/AppShellOverlays";
import { AppTopChrome } from "@/app/AppTopChrome";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useBackForwardControls } from "@/app/navigation/useBackForwardControls";
import { useMarkAsReadShortcuts } from "@/app/useMarkAsReadShortcuts";
import { useSettingsShortcuts } from "@/app/useSettingsShortcuts";
import { useWebviewZoomShortcuts } from "@/app/useWebviewZoomShortcuts";
import {
  channelsQueryKey,
  useChannelsQuery,
  useCreateChannelMutation,
  useHideDmMutation,
  useOpenDmMutation,
} from "@/features/channels/hooks";
import { useUnreadChannels } from "@/features/channels/useUnreadChannels";
import { useMembershipNotifications } from "@/features/channels/useMembershipNotifications";
import { getThreadReference } from "@/features/messages/lib/threading";
import { hasMentionForEvent } from "@/features/notifications/lib/shouldNotify";
import { useThreadFollows } from "@/features/messages/lib/useThreadFollows";
import {
  useHomeFeedNotifications,
  useHomeFeedNotificationState,
} from "@/features/notifications/hooks";
import {
  listenForDesktopNotificationActions,
  requestDockBounce,
  revealDesktopAppWindow,
  sendDesktopNotification,
  setDesktopAppBadge,
  type DesktopNotificationTarget,
} from "@/features/notifications/lib/desktop";
import {
  playNotificationSound,
  resolveSlotSound,
} from "@/features/notifications/lib/sound";
import { PreventSleepProvider } from "@/features/agents/usePreventSleep";
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
import { useProfileQuery } from "@/features/profile/hooks";
import {
  DEFAULT_SETTINGS_SECTION,
  type SettingsSection,
} from "@/features/settings/ui/SettingsPanels";
import { HuddleBar, HuddleProvider } from "@/features/huddle";
import { AppSidebar } from "@/features/sidebar/ui/AppSidebar";
import { useChannelMutes } from "@/features/sidebar/lib/useChannelMutes";
import { useChannelStars } from "@/features/sidebar/lib/useChannelStars";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { useApplyTemplate } from "@/features/channel-templates/useApplyTemplate";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useDeferredStartup } from "@/shared/hooks/useDeferredStartup";
import { joinChannel } from "@/shared/api/tauri";
import type { Channel, RelayEvent, SearchHit } from "@/shared/api/types";
import { ChannelNavigationProvider } from "@/shared/context/ChannelNavigationContext";
import { MainInsetProvider } from "@/shared/layout/MainInsetContext";
import { chromeCssVarDefaults } from "@/shared/layout/chromeLayout";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { useMessageDeepLinks } from "@/shared/useMessageDeepLinks";
import { SidebarInset, SidebarProvider } from "@/shared/ui/sidebar";

type AppView =
  | "home"
  | "channel"
  | "agents"
  | "workflows"
  | "pulse"
  | "projects";

const LazySettingsScreen = React.lazy(async () => {
  const module = await import("@/features/settings/ui/SettingsScreen");
  return { default: module.SettingsScreen };
});

const WINDOW_DRAG_HANDLE_HEIGHT = 44;
const WINDOW_DRAG_INTERACTIVE_SELECTOR =
  'button, a, input, textarea, select, [role="button"], [contenteditable="true"]';

function isWindowDragHandleEvent(event: MouseEvent | PointerEvent) {
  if (event.clientY > WINDOW_DRAG_HANDLE_HEIGHT) {
    return false;
  }

  const target = event.target;
  return !(
    target instanceof Element &&
    target.closest(WINDOW_DRAG_INTERACTIVE_SELECTOR)
  );
}

function toSearchHit(target: DesktopNotificationTarget): SearchHit | null {
  if (!target.eventId) {
    return null;
  }

  return {
    eventId: target.eventId,
    content: target.content ?? "",
    kind: target.kind ?? 9,
    pubkey: target.pubkey ?? "",
    channelId: target.channelId,
    channelName: target.channelName ?? null,
    createdAt: target.createdAt ?? Math.floor(Date.now() / 1_000),
    score: 0,
    threadRootId: target.threadRootId ?? null,
  };
}

function deriveShellRoute(pathname: string): {
  selectedChannelId: string | null;
  selectedView: AppView;
} {
  if (pathname.startsWith("/channels/")) {
    const [, , rawChannelId] = pathname.split("/");
    return {
      selectedChannelId: rawChannelId ? decodeURIComponent(rawChannelId) : null,
      selectedView: "channel",
    };
  }

  if (pathname === "/agents") {
    return {
      selectedChannelId: null,
      selectedView: "agents",
    };
  }

  if (pathname === "/workflows" || pathname.startsWith("/workflows/")) {
    return {
      selectedChannelId: null,
      selectedView: "workflows",
    };
  }

  if (pathname === "/projects" || pathname.startsWith("/projects/")) {
    return {
      selectedChannelId: null,
      selectedView: "projects",
    };
  }

  if (pathname === "/pulse") {
    return {
      selectedChannelId: null,
      selectedView: "pulse",
    };
  }

  return {
    selectedChannelId: null,
    selectedView: "home",
  };
}

export function AppShell() {
  useWebviewZoomShortcuts();

  const workspacesHook = useWorkspaces();
  const [isAddWorkspaceOpen, setIsAddWorkspaceOpen] = React.useState(false);

  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const [settingsSection, setSettingsSection] = React.useState<SettingsSection>(
    DEFAULT_SETTINGS_SECTION,
  );

  const [isChannelManagementOpen, setIsChannelManagementOpen] =
    React.useState(false);
  const [searchFocusRequest, setSearchFocusRequest] = React.useState(0);
  const [topbarSearchHidden, setTopbarSearchHidden] = React.useState(false);
  const [browseDialogType, setBrowseDialogType] =
    React.useState<BrowseDialogType>(null);
  const [isNewDmOpen, setIsNewDmOpen] = React.useState(false);
  const [isCreateChannelOpen, setIsCreateChannelOpen] = React.useState(false);
  const mainInsetRef = React.useRef<HTMLElement>(null);
  const location = useLocation();
  const queryClient = useQueryClient();
  const {
    goAgents,
    goChannel,
    goHome,
    goProjects,
    goPulse,
    goWorkflows,
    openSearchHit,
  } = useAppNavigation();
  const { canGoBack, canGoForward, goBack, goForward } =
    useBackForwardControls();
  const { selectedChannelId, selectedView } = React.useMemo(
    () => deriveShellRoute(location.pathname),
    [location.pathname],
  );

  const startupReady = useDeferredStartup();

  const identityQuery = useIdentityQuery();
  const { mutedChannelIds, muteChannel, unmuteChannel } = useChannelMutes(
    identityQuery.data?.pubkey,
  );
  const { starredChannelIds, starChannel, unstarChannel } = useChannelStars(
    identityQuery.data?.pubkey,
  );
  const profileQuery = useProfileQuery();
  const deferredPubkey = startupReady ? identityQuery.data?.pubkey : undefined;
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
  const refetchHomeFeedOnLiveMention = React.useEffectEvent(() => {
    void homeFeedQuery.refetch();
  });
  const handleChannelNotification = React.useEffectEvent(
    (_channelId: string, _event: RelayEvent) => {
      if (!notificationSettings.settings.desktopEnabled) return;
      void requestDockBounce();
    },
  );

  const handleDmNotification = React.useEffectEvent(
    (event: RelayEvent, channel: Channel) => {
      if (
        !notificationSettings.settings.desktopEnabled ||
        !notificationSettings.settings.slotAlertsEnabled.dm
      ) {
        return;
      }

      const channelName = channel.name?.trim() || "Direct message";
      const content = event.content.trim();
      const body =
        content.length > 0
          ? content.length > 140
            ? `${content.slice(0, 137).trimEnd()}...`
            : content
          : "New message";

      const threadRootId = getThreadReference(event.tags).rootId ?? null;

      void sendDesktopNotification({
        title: channelName,
        body,
        target: {
          channelId: channel.id,
          channelName,
          content: event.content,
          createdAt: event.created_at,
          eventId: event.id,
          kind: event.kind,
          pubkey: event.pubkey,
          threadRootId,
        },
      }).then((didSend) => {
        if (!didSend) return;
        playNotificationSound(
          resolveSlotSound(notificationSettings.settings, "dm"),
        );
        void requestDockBounce();
      });
    },
  );

  const channelsQuery = useChannelsQuery();
  const { refetch: refetchChannels } = channelsQuery;
  const channels = channelsQuery.data ?? [];
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

  const handleThreadReplyDesktopNotification = React.useEffectEvent(
    (channelId: string, event: RelayEvent) => {
      if (
        !notificationSettings.settings.desktopEnabled ||
        !notificationSettings.settings.slotAlertsEnabled.thread_reply
      ) {
        return;
      }

      // Replies that @-mention the user are owned by the home-feed mention
      // path — skip them here so they don't notify (and sound) twice.
      const pubkey = identityQuery.data?.pubkey?.trim().toLowerCase() ?? "";
      if (hasMentionForEvent(event, pubkey)) {
        return;
      }

      const channel = channels.find((entry) => entry.id === channelId);
      const channelName = channel?.name?.trim() || "Thread";
      const content = event.content.trim();
      const body =
        content.length > 0
          ? content.length > 140
            ? `${content.slice(0, 137).trimEnd()}...`
            : content
          : "New reply";

      const threadRootId = getThreadReference(event.tags).rootId ?? null;

      void sendDesktopNotification({
        title: `Reply in ${channelName}`,
        body,
        target: {
          channelId,
          channelName,
          content: event.content,
          createdAt: event.created_at,
          eventId: event.id,
          kind: event.kind,
          pubkey: event.pubkey,
          threadRootId,
        },
      }).then((didSend) => {
        if (!didSend) return;
        playNotificationSound(
          resolveSlotSound(notificationSettings.settings, "thread_reply"),
        );
        void requestDockBounce();
      });
    },
  );

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
    highPriorityUnreadChannelIds,
    getEffectiveTimestamp: getChannelReadAt,
    readStateVersion,
    participatedRootIds,
    authoredRootIds,
    threadActivityItems,
    mutedRootIds,
    muteThread,
    unmuteThread,
  } = useUnreadChannels(
    sidebarChannels,
    activeChannel,
    // Wait for ChannelScreen to report the latest loaded message before
    // advancing unread state for the active channel.
    null,
    {
      pubkey: identityQuery.data?.pubkey,
      relayClient,
      currentPubkey: identityQuery.data?.pubkey,
      mutedChannelIds,
      notifyForActiveChannel: notificationSettings.settings.notifyWhileViewing,
      onChannelMessage: handleChannelNotification,
      onDmMessage: handleDmNotification,
      onLiveMention: refetchHomeFeedOnLiveMention,
      onThreadReplyDesktopNotification: handleThreadReplyDesktopNotification,
      followedRootIds,
    },
  );

  // Badge count is computed here (rather than inside useHomeFeedNotifications)
  // so it can consume the NIP-RS read-state lifted from the single
  // ReadStateManager mounted via useUnreadChannels above. Channel-backed
  // feed items contribute to the badge iff strictly newer than that
  // channel's read marker; non-channel items keep their seen-set fallback.
  const { homeBadgeCount, homeBadgeCountExcludingHighPriority } =
    useHomeFeedNotificationState(
      homeFeedQuery.data,
      identityQuery.data?.pubkey,
      notificationSettings.settings,
      notificationSettings.setDesktopEnabled,
      selectedView === "home",
      getChannelReadAt,
      readStateVersion,
      highPriorityUnreadChannelIds,
      feedProfilesQuery.data?.profiles,
      mutedChannelIds,
    );

  const isNotifiedForThread = React.useCallback(
    (rootId: string) =>
      !mutedRootIds.has(rootId) &&
      (followedRootIds.has(rootId) ||
        participatedRootIds.has(rootId) ||
        authoredRootIds.has(rootId)),
    [followedRootIds, mutedRootIds, participatedRootIds, authoredRootIds],
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
  const handleOpenBrowseForums = React.useCallback(() => {
    setBrowseDialogType("forum");
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
      setSettingsSection(section);
      setSettingsOpen(true);
    },
    [],
  );

  const handleCloseSettings = React.useCallback(() => {
    setSettingsOpen(false);
  }, []);

  const handleOpenSearchResult = React.useCallback(
    (hit: SearchHit) => {
      void openSearchHit(hit);
    },
    [openSearchHit],
  );

  const handleDesktopNotificationAction = React.useEffectEvent(
    async (target: DesktopNotificationTarget) => {
      await revealDesktopAppWindow();

      if (!target.channelId) {
        void goHome();
        return;
      }

      const anchor = toSearchHit(target);
      if (!anchor) {
        await goChannel(target.channelId);
        return;
      }

      await openSearchHit(anchor);
    },
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
    const numericCount =
      highPriorityUnreadChannelIds.size + homeBadgeCountExcludingHighPriority;
    if (numericCount > 0) {
      void setDesktopAppBadge({ kind: "count", count: numericCount });
    } else if (unreadChannelIds.size > 0) {
      void setDesktopAppBadge({ kind: "dot" });
    } else {
      void setDesktopAppBadge({ kind: "none" });
    }
  }, [
    homeBadgeCountExcludingHighPriority,
    highPriorityUnreadChannelIds.size,
    unreadChannelIds.size,
  ]);

  // Dispatch `buzz://message` deep links into the router.
  useMessageDeepLinks();

  React.useEffect(() => {
    let isCancelled = false;
    let cleanup = () => {};

    void listenForDesktopNotificationActions((target) => {
      if (isCancelled) {
        return;
      }

      void handleDesktopNotificationAction(target);
    }).then((dispose) => {
      if (isCancelled) {
        dispose();
        return;
      }

      cleanup = dispose;
    });

    return () => {
      isCancelled = true;
      cleanup();
    };
  }, []);

  const handleOpenNewDm = React.useCallback(() => {
    setIsNewDmOpen(true);
  }, []);

  const handleOpenCreateChannel = React.useCallback(() => {
    setIsCreateChannelOpen(true);
  }, []);

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

  React.useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (event.button !== 0 || event.detail > 1) {
        return;
      }

      if (!isWindowDragHandleEvent(event)) {
        return;
      }

      void getCurrentWindow().startDragging();
    }

    function handleDoubleClick(event: MouseEvent) {
      if (event.button !== 0 || !isWindowDragHandleEvent(event)) {
        return;
      }

      event.preventDefault();
      void getCurrentWindow().toggleMaximize();
    }

    window.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("dblclick", handleDoubleClick, true);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("dblclick", handleDoubleClick, true);
    };
  }, []);

  return (
    <PreventSleepProvider>
      <ChannelNavigationProvider channels={channels}>
        <AppShellProvider
          value={{
            markAllChannelsRead,
            markChannelRead,
            markChannelUnread,
            openCreateChannel: handleOpenCreateChannel,
            openChannelManagement: () => {
              setIsChannelManagementOpen(true);
            },
            getChannelReadAt,
            readStateVersion,
            followThread: handleFollowThread,
            unfollowThread: handleUnfollowThread,
            isFollowingThread,
            isNotifiedForThread,
            setTopbarSearchHidden,
            threadActivityItems,
          }}
        >
          <HuddleProvider>
            <div className="flex h-dvh flex-col overflow-hidden overscroll-none">
              <SidebarProvider className="min-h-0 flex-1 overflow-hidden">
                {!settingsOpen ? (
                  <AppTopChrome
                    canGoBack={canGoBack}
                    canGoForward={canGoForward}
                    channels={channels}
                    currentPubkey={identityQuery.data?.pubkey}
                    onGoBack={goBack}
                    onGoForward={goForward}
                    onOpenChannel={(channelId) => {
                      void goChannel(channelId);
                    }}
                    onOpenResult={handleOpenSearchResult}
                    searchHidden={topbarSearchHidden}
                    searchFocusRequest={searchFocusRequest}
                  />
                ) : null}
                {settingsOpen ? (
                  <React.Suspense fallback={null}>
                    <LazySettingsScreen
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
                      onSectionChange={setSettingsSection}
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
                ) : (
                  <>
                    <AppSidebar
                      activeWorkspace={workspacesHook.activeWorkspace}
                      channels={sidebarChannels}
                      currentPubkey={identityQuery.data?.pubkey}
                      errorMessage={
                        channelsQuery.error instanceof Error
                          ? channelsQuery.error.message
                          : undefined
                      }
                      fallbackDisplayName={identityQuery.data?.displayName}
                      homeBadgeCount={homeBadgeCount}
                      isAddWorkspaceOpen={isAddWorkspaceOpen}
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

                        await applyCanvas(templateId, createdChannel.id, name);
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

                        await applyCanvas(templateId, createdForum.id, name);
                        await goChannel(createdForum.id);
                        void applyAgents(templateId, createdForum.id);
                      }}
                      onHideDm={handleHideDm}
                      onMarkAllChannelsRead={markAllChannelsRead}
                      onMarkChannelRead={markChannelRead}
                      onMarkChannelUnread={markChannelUnread}
                      onOpenBrowseChannels={handleOpenBrowseChannels}
                      onOpenBrowseForums={handleOpenBrowseForums}
                      onOpenDm={async ({ pubkeys }) => {
                        const directMessage = await openDmMutation.mutateAsync({
                          pubkeys,
                        });
                        await goChannel(directMessage.id);
                      }}
                      onSelectAgents={() => {
                        void goAgents();
                      }}
                      onSelectChannel={(channelId) => {
                        void goChannel(channelId);
                      }}
                      onSelectHome={() => {
                        void goHome();
                      }}
                      onSelectProjects={() => {
                        void goProjects();
                      }}
                      onSelectPulse={() => {
                        void goPulse();
                      }}
                      onSelectSettings={handleOpenSettings}
                      onSelectWorkflows={() => {
                        void goWorkflows();
                      }}
                      onSetPresenceStatus={(status) =>
                        presenceSession.setStatus(status)
                      }
                      onSetUserStatus={(text, emoji) =>
                        setUserStatusMutation.mutate({ text, emoji })
                      }
                      onClearUserStatus={() =>
                        setUserStatusMutation.mutate({ text: "", emoji: "" })
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
                        className="min-h-0 min-w-0 overflow-hidden"
                        style={chromeCssVarDefaults}
                      >
                        <Outlet />
                      </SidebarInset>
                    </MainInsetProvider>
                  </>
                )}

                <AppShellOverlays
                  activeChannel={activeChannel}
                  browseDialogType={browseDialogType}
                  channels={channels}
                  currentPubkey={identityQuery.data?.pubkey}
                  isChannelManagementOpen={isChannelManagementOpen}
                  onBrowseChannelJoin={handleBrowseChannelJoin}
                  onBrowseDialogOpenChange={handleBrowseDialogOpenChange}
                  onChannelManagementOpenChange={setIsChannelManagementOpen}
                  onDeleteActiveChannel={() => {
                    setIsChannelManagementOpen(false);
                    void goHome({ replace: true });
                  }}
                  onSelectChannel={(channelId) => {
                    void goChannel(channelId);
                  }}
                />
              </SidebarProvider>
              <HuddleBar />
            </div>
          </HuddleProvider>
        </AppShellProvider>
      </ChannelNavigationProvider>
    </PreventSleepProvider>
  );
}

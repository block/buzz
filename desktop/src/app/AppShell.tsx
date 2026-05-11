import { ChevronLeft, ChevronRight } from "lucide-react";
import * as React from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useQueryClient } from "@tanstack/react-query";
import { Outlet, useLocation } from "@tanstack/react-router";

import { AppShellProvider } from "@/app/AppShellContext";
import {
  AppShellOverlays,
  type BrowseDialogType,
} from "@/app/AppShellOverlays";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useBackForwardControls } from "@/app/navigation/useBackForwardControls";
import { useWebviewZoomShortcuts } from "@/app/useWebviewZoomShortcuts";
import {
  channelsQueryKey,
  useChannelsQuery,
  useCreateChannelMutation,
  useHideDmMutation,
  useOpenDmMutation,
} from "@/features/channels/hooks";
import { useUnreadChannels } from "@/features/channels/useUnreadChannels";
import { useHomeFeedNotifications } from "@/features/notifications/hooks";
import {
  listenForDesktopNotificationActions,
  revealDesktopAppWindow,
  sendDesktopNotification,
  setDesktopAppBadgeCount,
  type DesktopNotificationTarget,
} from "@/features/notifications/lib/desktop";
import { playNotificationSound } from "@/features/notifications/lib/sound";
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
import { useProfileQuery } from "@/features/profile/hooks";
import {
  DEFAULT_SETTINGS_SECTION,
  type SettingsSection,
} from "@/features/settings/ui/SettingsPanels";
import { HuddleBar, HuddleProvider } from "@/features/huddle";
import { AppSidebar } from "@/features/sidebar/ui/AppSidebar";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useDeferredStartup } from "@/shared/hooks/useDeferredStartup";
import { joinChannel } from "@/shared/api/tauri";
import type { Channel, RelayEvent, SearchHit } from "@/shared/api/types";
import { ChannelNavigationProvider } from "@/shared/context/ChannelNavigationContext";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { Button } from "@/shared/ui/button";
import {
  SidebarInset,
  SidebarProvider,
  SidebarTrigger,
} from "@/shared/ui/sidebar";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";

type AppView = "home" | "channel" | "agents" | "workflows" | "pulse";

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
  const [isSearchOpen, setIsSearchOpen] = React.useState(false);
  const [browseDialogType, setBrowseDialogType] =
    React.useState<BrowseDialogType>(null);
  const [isNewDmOpen, setIsNewDmOpen] = React.useState(false);
  const location = useLocation();
  const queryClient = useQueryClient();
  const { goAgents, goChannel, goHome, goPulse, goWorkflows, openSearchHit } =
    useAppNavigation();
  const { canGoBack, canGoForward, goBack, goForward } =
    useBackForwardControls();
  const { selectedChannelId, selectedView } = React.useMemo(
    () => deriveShellRoute(location.pathname),
    [location.pathname],
  );

  const startupReady = useDeferredStartup();

  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const deferredPubkey = startupReady ? identityQuery.data?.pubkey : undefined;
  usePresenceSubscription();
  useUserStatusSubscription();
  const presenceSession = usePresenceSession(deferredPubkey);
  const selfStatusQuery = useUserStatusQuery(
    deferredPubkey ? [deferredPubkey] : [],
  );
  const setUserStatusMutation = useSetUserStatusMutation(deferredPubkey);
  const { homeBadgeCount, homeFeedQuery, notificationSettings } =
    useHomeFeedNotifications(
      identityQuery.data?.pubkey,
      selectedView === "home",
    );
  const refetchHomeFeedOnLiveMention = React.useEffectEvent(() => {
    void homeFeedQuery.refetch();
  });

  const handleDmNotification = React.useEffectEvent(
    (event: RelayEvent, channel: Channel) => {
      if (!notificationSettings.settings.desktopEnabled) {
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
        },
      }).then((didSend) => {
        if (didSend && notificationSettings.settings.soundEnabled) {
          playNotificationSound();
        }
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

  const { markChannelRead, unreadChannelIds } = useUnreadChannels(
    channels,
    activeChannel,
    // Wait for ChannelScreen to report the latest loaded message before
    // advancing unread state for the active channel.
    null,
    {
      pubkey: identityQuery.data?.pubkey,
      relayClient,
      currentPubkey: identityQuery.data?.pubkey,
      onDmMessage: handleDmNotification,
      onLiveMention: refetchHomeFeedOnLiveMention,
    },
  );

  const createChannelMutation = useCreateChannelMutation();
  const createForumMutation = useCreateChannelMutation();
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
    setIsSearchOpen(true);
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
      setIsSearchOpen(false);
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
    void setDesktopAppBadgeCount(homeBadgeCount);
  }, [homeBadgeCount]);

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
    handleOpenSearch,
    goHome,
    settingsOpen,
  ]);

  React.useLayoutEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const isSettingsShortcut =
        (event.key === "," || event.code === "Comma") &&
        hasPrimaryShortcutModifier(event) &&
        !event.altKey &&
        !event.shiftKey;

      if (!isSettingsShortcut) {
        return;
      }

      event.preventDefault();
      if (settingsOpen) {
        handleCloseSettings();
        return;
      }

      handleOpenSettings();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [handleCloseSettings, handleOpenSettings, settingsOpen]);

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
            markChannelRead,
            openChannelManagement: () => {
              setIsChannelManagementOpen(true);
            },
          }}
        >
          <HuddleProvider>
            <div className="flex h-dvh flex-col overflow-hidden overscroll-none">
              <SidebarProvider className="min-h-0 flex-1 overflow-hidden">
                <div
                  aria-hidden="true"
                  className="fixed inset-x-0 top-0 z-20 h-10 cursor-default select-none"
                  data-tauri-drag-region
                />
                <div className="fixed left-[80px] top-[9px] z-50 flex items-center gap-0.5">
                  <SidebarTrigger className="h-[22px] w-[22px] text-muted-foreground/70 hover:bg-muted/60 hover:text-foreground" />
                  <Button
                    aria-label="Go back"
                    className="h-[22px] w-[22px] text-muted-foreground/70 hover:bg-muted/60 hover:text-foreground"
                    data-testid="global-back"
                    disabled={!canGoBack}
                    onClick={goBack}
                    size="icon"
                    variant="ghost"
                  >
                    <ChevronLeft className="h-3 w-3" />
                  </Button>
                  <Button
                    aria-label="Go forward"
                    className="h-[22px] w-[22px] text-muted-foreground/70 hover:bg-muted/60 hover:text-foreground"
                    data-testid="global-forward"
                    disabled={!canGoForward}
                    onClick={goForward}
                    size="icon"
                    variant="ghost"
                  >
                    <ChevronRight className="h-3 w-3" />
                  </Button>
                </div>
                <div className="fixed right-[16px] top-[8px] z-50">
                  <UpdateIndicator
                    onOpenUpdates={() => handleOpenSettings("updates")}
                  />
                </div>
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
                  isPresencePending={presenceSession.isPending}
                  onAddWorkspace={(workspace) => {
                    const id = workspacesHook.addWorkspace(workspace);
                    workspacesHook.switchWorkspace(id);
                  }}
                  onAddWorkspaceOpenChange={setIsAddWorkspaceOpen}
                  onNewDmOpenChange={setIsNewDmOpen}
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
                  }) => {
                    const createdChannel =
                      await createChannelMutation.mutateAsync({
                        name,
                        description,
                        channelType: "stream",
                        visibility,
                        ttlSeconds,
                      });

                    await goChannel(createdChannel.id);
                  }}
                  onCreateForum={async ({
                    description,
                    name,
                    visibility,
                    ttlSeconds,
                  }) => {
                    const createdForum = await createForumMutation.mutateAsync({
                      name,
                      description,
                      channelType: "forum",
                      visibility,
                      ttlSeconds,
                    });

                    await goChannel(createdForum.id);
                  }}
                  onHideDm={handleHideDm}
                  onOpenBrowseChannels={handleOpenBrowseChannels}
                  onOpenBrowseForums={handleOpenBrowseForums}
                  onOpenDm={async ({ pubkeys }) => {
                    const directMessage = await openDmMutation.mutateAsync({
                      pubkeys,
                    });
                    await goChannel(directMessage.id);
                  }}
                  onOpenSearch={handleOpenSearch}
                  onSelectAgents={() => {
                    void goAgents();
                  }}
                  onSelectChannel={(channelId) => {
                    void goChannel(channelId);
                  }}
                  onSelectHome={() => {
                    void goHome();
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
                      ? (selfStatusQuery.data?.[deferredPubkey.toLowerCase()] ??
                        undefined)
                      : undefined
                  }
                  selectedChannelId={selectedChannelId}
                  selectedView={selectedView}
                  unreadChannelIds={unreadChannelIds}
                />

                <SidebarInset className="min-h-0 min-w-0 overflow-hidden">
                  <Outlet />
                </SidebarInset>

                <AppShellOverlays
                  activeChannel={activeChannel}
                  browseDialogType={browseDialogType}
                  channels={channels}
                  currentPubkey={identityQuery.data?.pubkey}
                  isChannelManagementOpen={isChannelManagementOpen}
                  isSearchOpen={isSearchOpen}
                  onBrowseChannelJoin={handleBrowseChannelJoin}
                  onBrowseDialogOpenChange={handleBrowseDialogOpenChange}
                  onChannelManagementOpenChange={setIsChannelManagementOpen}
                  onDeleteActiveChannel={() => {
                    setIsChannelManagementOpen(false);
                    void goHome({ replace: true });
                  }}
                  onOpenSearchResult={handleOpenSearchResult}
                  onSearchOpenChange={setIsSearchOpen}
                  onSelectChannel={(channelId) => {
                    void goChannel(channelId);
                  }}
                />

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
                      onSetMentionNotificationsEnabled={
                        notificationSettings.setMentionsEnabled
                      }
                      onSetNeedsActionNotificationsEnabled={
                        notificationSettings.setNeedsActionEnabled
                      }
                      onSetSoundEnabled={notificationSettings.setSoundEnabled}
                      section={settingsSection}
                    />
                  </React.Suspense>
                ) : null}
              </SidebarProvider>
              <HuddleBar />
            </div>
          </HuddleProvider>
        </AppShellProvider>
      </ChannelNavigationProvider>
    </PreventSleepProvider>
  );
}

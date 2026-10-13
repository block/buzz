import * as React from "react";
import { ProtectedGlobalOverlay } from "@protected-feature-components";
import { useQueryClient } from "@tanstack/react-query";
import { Outlet, useLocation, useNavigate, useRouter } from "@tanstack/react-router";
import {
  deriveShellRoute,
  markAllReadSources,
  shouldPersistLinkSidePanel,
} from "@/app/AppShell.helpers";
import { useTerminalContext } from "@/app/useTerminalContext";
import { AppShellProvider } from "@/app/AppShellContext";
import { AppShellOverlays, TerminalBootstrap } from "@/app/AppShellOverlays";
import { AppShellChannelSurface } from "@/app/AppShellChannelSurface";
import { AppHuddleShell } from "@/app/AppHuddleShell";
import { AppTopChrome } from "@/app/AppTopChrome";
import { SurfaceTabsProvider } from "@/app/surfaceTabs/SurfaceTabsProvider";
import { usePopoutBootstrap } from "@/app/usePopoutBootstrap";
import {
  type TerminalContextOverride,
  TerminalContextOverrideProvider,
} from "@/app/TerminalContextOverrideContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useBackForwardControls } from "@/app/navigation/useBackForwardControls";
import { useCommunityNavigationTransitions } from "@/app/useCommunityNavigationTransitions";
import { useLiveHomeFeedActions } from "@/app/useLiveHomeFeedActions";
import { useLiveProjectWorkItems } from "@/features/projects/useLiveProjectWorkItems";
import { useChannelBrowserDialog } from "@/app/useChannelBrowserDialog";
import { useMarkAsReadShortcuts } from "@/app/useMarkAsReadShortcuts";
import { useSettingsShortcuts } from "@/app/useSettingsShortcuts";
import { useAppShellKeyboardShortcuts } from "@/app/useAppShellKeyboardShortcuts";
import { useAppShellDesktopNotifications } from "@/app/useAppShellDesktopNotifications";
import { useAppShellLifecycleEffects } from "@/app/useAppShellLifecycleEffects";
import { useChannelActivityProjection } from "@/app/useChannelActivityProjection";
import { useTauriWindowDrag } from "@/app/useTauriWindowDrag";
import { useWebviewZoomShortcuts } from "@/app/useWebviewZoomShortcuts";
import { useHuddlePresentation } from "@/app/useHuddlePresentation";
import { useUserSignerDraftListener } from "@/features/term-session/lib/useUserSignerDraftListener";
import {
  isArchivedHuddleSidebarChannel,
  shouldShowSidebarChannel,
  sortArchivedHuddleChannels,
} from "@/app/huddleChannelVisibility";
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
import { useThreadStars } from "@/features/sidebar/lib/useThreadStars";
import {
  useHomeFeedNotifications,
  useHomeFeedNotificationState,
} from "@/features/notifications/hooks";
import { PreventSleepProvider } from "@/features/agents/usePreventSleep";
import { requestOpenCreateAgent } from "@/features/agents/openCreateAgentEvent";
import { useAgentsDataRefresh } from "@/features/agents/lib/useAgentsDataRefresh";
import { useManagedAgentRuntimeReconciliation } from "@/features/agents/useManagedAgentRuntimeReconciliation";
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
  visibleUserStatus,
  useUserStatusSubscription,
} from "@/features/user-status/hooks";
import { useCommunityEmojiLiveUpdates } from "@/features/custom-emoji/hooks";
import { useRemoteAccessEvents } from "@/features/bridge/useRemoteAccessEvents";
import { RemoteBrowserIndicator } from "@/features/bridge/ui/RemoteBrowserIndicator";
import {
  useCommunityPinnedSitesLiveUpdates,
} from "@/features/pinned-sites/hooks";
import { closeLinkSidePanel } from "@/features/link-panel/lib/linkSidePanelStore";
import { hideAllPinWebviews } from "@/features/pinned-sites/lib/pinWebview";
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
import { useReminderNotifications } from "@/features/reminders/useReminderNotifications";
import { AppSidebar } from "@/features/sidebar/ui/AppSidebar";
import { requestFocusedThreadClose } from "@/features/channels/focusedThreadCloseRequest";
import { CommunityRail } from "@/features/sidebar/ui/CommunityRail";
import { useChannelMutes } from "@/features/sidebar/lib/useChannelMutes";
import { useChannelStars } from "@/features/sidebar/lib/useChannelStars";
import { useBridgePublisher } from "@/features/bridge/useBridgePublisher";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  consumePendingCommunityRestore,
  loadCommunityDestination,
  saveCommunityDestination,
} from "@/features/communities/communityNavigationStorage";
import { useAddCommunityDialogState } from "@/features/communities/addCommunityPrefill";
import { useApplyTemplate } from "@/features/channel-templates/useApplyTemplate";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useRelayAutoHeal } from "@/shared/api/useRelayAutoHeal";
import { useDeferredStartup } from "@/shared/hooks/useDeferredStartup";
import { useWebviewScrollBoundaryLock } from "@/shared/hooks/useWebviewScrollBoundaryLock";
import { joinChannel } from "@/shared/api/tauri";
import type { Channel, ChannelVisibility, SearchHit } from "@/shared/api/types";
import { ChannelNavigationProvider } from "@/shared/context/ChannelNavigationContext";
import { useAppDeepLinks } from "@/shared/useAppDeepLinks";
import { SidebarProvider } from "@/shared/ui/sidebar";
import { usePhoneLayout } from "@/shared/lib/phoneLayout";
import { PhoneMeScreen } from "@/app/phone/PhoneMeScreen";
import { PhoneTabBar } from "@/app/phone/PhoneTabBar";
import { PhoneTabSwitcher } from "@/app/phone/PhoneTabSwitcher";
import { TopbarSearch } from "@/features/search/ui/TopbarSearch";
import {
  type PhoneTab,
  overlayFromHistoryState,
  phoneHidesTabBar,
  phoneTabForPath,
  registerPhoneOverlayNavigator,
  setPhoneOverlay,
  syncPhoneOverlayFromHistory,
  usePhoneOverlay,
} from "@/app/phone/phoneTabs";
import { PhoneNavRow } from "@/app/phone/PhoneNavRow";
import { RelayConnectionOverlay } from "@/app/RelayConnectionOverlay";
import { useSidebarRelayConnectionCard } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { AppShellTrayMenu } from "@/app/useAppShellTrayMenu";
import { AppProfilePanelProvider } from "@/app/AppProfilePanelProvider";
import { AppWorkflowEditorOverlayProvider } from "@/app/AppWorkflowEditorOverlayProvider";
import { LazySettingsScreen } from "@/app/LazySettingsScreen";
import { useInboxPushPairing, useIsNotifiedForThread } from "@/app/useInboxPushPairing";
const EMPTY_CHANNELS: Channel[] = [];
export function AppShell() {
  useWebviewZoomShortcuts();
  useTauriWindowDrag();
  useWebviewScrollBoundaryLock();
  const communitiesHook = useCommunities();
  const {
    activeHuddleChannelId,
    handleHuddleCompanionOpen,
    handleHuddleEnded,
    handleHuddleStartPendingChange,
    handleHuddleStarted,
    handleHuddleVisibilityChange,
    handleSidebarChannelSelect,
    huddleBackingChannelIds,
    revealedHuddleChannelIds,
    isHuddleCompanionOpen,
    isHuddleDrawerOpen,
    isHuddleRoom,
    isHuddleRoomStarting,
    presentation: huddlePresentation,
    showHuddleInMainApp,
    viewHuddleChannel,
  } = useHuddlePresentation();

  useUserSignerDraftListener(true);
  const popout = usePopoutBootstrap();
  const hideAppChrome = isHuddleRoom || popout != null;
  const hasCommunityRail = communitiesHook.communities.length > 1;
  const addCommunityDialog = useAddCommunityDialogState();
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
  const location = useLocation();
  const queryClient = useQueryClient();
  useManagedAgentRuntimeReconciliation(communitiesHook.communities); // sync storage snapshot
  const {
    goAgents,
    goBrowsers,
    goBots,
    goPinnedSite,
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
  const { selectedChannelId, selectedPinId, selectedView } = React.useMemo(
    () => deriveShellRoute(location.pathname),
    [location.pathname],
  );
  const {
    leaveAndRemoveCommunity: handleLeaveCommunity,
    removeCommunityFromDevice: removeFromDevice,
    switchCommunity: handleSwitchCommunity,
  } = useCommunityNavigationTransitions({
    communities: communitiesHook,
    goHome,
    selectedChannelId,
    selectedView,
  });
  // Settings lives in history so back returns to the previous app entry.
  const settingsOpen = location.pathname === "/settings";
  // Phone layout (< 600px, web): bottom tab bar replaces the slide-out left nav.
  const isPhone = usePhoneLayout();
  const phoneOverlay = usePhoneOverlay();
  const phoneTab = phoneTabForPath(location.pathname, phoneOverlay);
  const showPhoneTabBar =
    isPhone && !phoneHidesTabBar(location.pathname, phoneOverlay);
  const navigate = useNavigate();
  const router = useRouter();
  // A thread opened inside a channel (?thread=…) is its own phone screen: Back closes it via the panel's own handler.
  const isPhoneInlineThread =
    location.pathname.startsWith("/channels/") &&
    !location.pathname.includes("/posts/") &&
    Boolean((location.search as { thread?: unknown }).thread);
  const handlePhoneNavBack = React.useCallback(() => {
    if (isPhoneInlineThread) {
      const close = document.querySelector<HTMLElement>(
        '[data-testid="message-thread-back"], [data-testid="message-thread-panel"] [data-testid="auxiliary-panel-close"]',
      );
      if (close) {
        close.click();
        return;
      }
    }
    handlePhoneNavHistoryRef.current(-1);
  }, [isPhoneInlineThread]);
  const handlePhoneNavHistoryRef = React.useRef<(dir: -1 | 1) => void>(() => goBack());
  // Phone list screens (Channels / Me / Search) are history entries: the overlay rides in the entry's state, so Back,
  // Forward and Settings' Back step through them (see phoneTabs.ts). Clearing an overlay is deferred a tick because a
  // navigation almost always follows (opening a channel from the list); that new entry already has no overlay.
  const historyOverlay = overlayFromHistoryState(location.state, router.history.location.href);
  React.useEffect(() => {
    if (isPhone) syncPhoneOverlayFromHistory(historyOverlay);
  }, [historyOverlay, isPhone]);
  React.useEffect(() => {
    if (!isPhone) return;
    const write = (next: ReturnType<typeof overlayFromHistoryState>, replace: boolean) => {
      const loc = router.history.location;
      const { __TSR_index: _i, __TSR_key: _k, key: _key, ...rest } = (loc.state ?? {}) as unknown as Record<string, unknown>;
      const state = { ...rest, phoneOverlay: next, phoneOverlayHref: loc.href } as never;
      if (replace) router.history.replace(loc.href, state);
      else router.history.push(loc.href, state);
    };
    return registerPhoneOverlayNavigator((next, opts) => {
      const loc = router.history.location;
      if (overlayFromHistoryState(loc.state, loc.href) === next) return;
      if (next === null && !opts?.replace) {
        // Usually a navigation follows (a channel tapped in the list) and its new entry has no overlay, so nothing
        // needs writing. Only when no navigation lands (and none is still loading) does the list close in place.
        const href = loc.href;
        const settle = (tries: number) =>
          window.setTimeout(() => {
            const now = router.history.location;
            if (now.href !== href || overlayFromHistoryState(now.state, now.href) === null) return;
            if (router.state.isLoading && tries > 0) return settle(tries - 1);
            write(null, false);
          }, 250);
        settle(8);
        return;
      }
      write(next, Boolean(opts?.replace));
    });
  }, [isPhone, router]);
  // Linear phone breadcrumb over the real browser history (same entries desktop's Back/Forward walk). Each entry's kind
  // is remembered per history index in sessionStorage (survives reloads): "list" entries (the Channels / Search list
  // screens) are pass-through, so Back from #design lands on #general rather than ping-ponging through the list
  // between them; consecutive duplicates (same href + screen) are skipped too. A new navigation after Back replaces
  // the entry at that index, which truncates the remembered forward entries (the browser already dropped them).
  const navIndex = ((location.state as { __TSR_index?: number } | undefined)?.__TSR_index ?? 0) as number;
  const navKey = String((location.state as { __TSR_key?: string } | undefined)?.__TSR_key ?? navIndex);
  const navSig = `${router.history.location.href}|${historyOverlay ?? ""}`;
  React.useEffect(() => {
    if (!isPhone) return;
    const map = readPhoneNavMap();
    if (map[navIndex] && map[navIndex].key !== navKey) {
      for (const k of Object.keys(map)) if (Number(k) >= navIndex) delete map[Number(k)];
    }
    map[navIndex] = {
      key: navKey,
      sig: navSig,
      list: historyOverlay === "channels" || historyOverlay === "search",
    };
    writePhoneNavMap(map);
  }, [historyOverlay, isPhone, navIndex, navKey, navSig]);
  const phoneStep = React.useCallback(
    (dir: -1 | 1): number => {
      const map = readPhoneNavMap();
      const here = map[navIndex]?.sig ?? navSig;
      let t = navIndex + dir;
      const limit = dir < 0 ? 0 : Math.max(...Object.keys(map).map(Number), navIndex);
      while ((dir < 0 ? t > limit : t < limit) && (map[t]?.list || map[t]?.sig === here)) t += dir;
      if (t < 0 || (dir > 0 && t > limit)) return 0;
      return t - navIndex;
    },
    [navIndex, navSig],
  );
  const handlePhoneNavHistory = React.useCallback(
    (dir: -1 | 1) => {
      const steps = phoneStep(dir);
      if (steps === 0) return dir < 0 ? goBack() : goForward();
      router.history.go(steps);
    },
    [goBack, goForward, phoneStep, router],
  );
  handlePhoneNavHistoryRef.current = handlePhoneNavHistory;
  const handlePhoneNavHome = React.useCallback(() => {
    if (location.pathname === "/settings") {
      router.history.push("/", { phoneOverlay: "channels", phoneOverlayHref: "/" } as never);
      return;
    }
    setPhoneOverlay("channels");
  }, [location.pathname, router]);
  // Phone opens on the Channels tab (the root route is Inbox on iPad/desktop).
  const phoneStartedRef = React.useRef(false);
  React.useEffect(() => {
    if (!isPhone || phoneStartedRef.current) return;
    phoneStartedRef.current = true;
    if (location.pathname === "/" && !historyOverlay) setPhoneOverlay("channels", { replace: true });
  }, [historyOverlay, isPhone, location.pathname]);
  React.useEffect(() => {
    const root = document.documentElement;
    if (showPhoneTabBar) root.setAttribute("data-phone-tabbar", "");
    else root.removeAttribute("data-phone-tabbar");
    return () => root.removeAttribute("data-phone-tabbar");
  }, [showPhoneTabBar]);
  const rawLocationSearchSection = (location.search as { section?: unknown })
    .section;
  // Migrate the legacy "moderation" token to "relay-admin" (renamed section
  // id). useLocation().search is the raw URL query, bypassing route validation,
  // so the alias must be applied here too. The "doctor" alias is NOT needed on
  // this path: AppShell never carried it on main, and it lives only in
  // validateSettingsSearch where route validation rewrites it before rendering.
  const locationSearchSection =
    rawLocationSearchSection === "moderation"
      ? "relay-admin"
      : rawLocationSearchSection;
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
  useAutoRestartPolicy(communitiesHook.activeCommunity?.relayUrl);
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
  // The tasks page hides the rail, so task movement has to be followed here.
  useLiveProjectWorkItems();
  usePresenceSubscription();
  useUserStatusSubscription();
  useCommunityEmojiLiveUpdates();
  useCommunityPinnedSitesLiveUpdates();
  React.useEffect(() => {
    if (selectedView !== "pin") {
      void hideAllPinWebviews();
    }
    // Tear down fullscreen link web when leaving hosts that render it
    // (channel + Inbox). Keep it on those views so message links can open.
    if (!shouldPersistLinkSidePanel(selectedView)) {
      closeLinkSidePanel();
    }
  }, [selectedView]);
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
  useRemoteAccessEvents(identityQuery.data?.pubkey);
  // PWA bridge (slice A): desktop publishes agent status for the member's own browser. No-op outside Tauri.
  useBridgePublisher(
    identityQuery.data?.pubkey,
    communitiesHook.activeCommunity?.relayUrl,
    (id) => channels.find((c) => c.id === id)?.name,
    channelsQuery.data,
  );
  const dmChannelIds = React.useMemo(() => {
    const ids = new Set<string>();
    for (const channel of channelsQuery.data ?? []) {
      if (channel.channelType === "dm") ids.add(channel.id);
    }
    return ids;
  }, [channelsQuery.data]);
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
    `${communitiesHook.activeCommunity?.id ?? "none"}-${communitiesHook.reinitKey}`,
  );
  const memberChannels = React.useMemo(
    () => channels.filter((channel) => channel.isMember),
    [channels],
  );
  const sidebarChannels = React.useMemo(
    () =>
      memberChannels.filter(
        (channel) =>
          channel.archivedAt === null &&
          shouldShowSidebarChannel(
            channel,
            huddleBackingChannelIds,
            revealedHuddleChannelIds,
          ),
      ),
    [huddleBackingChannelIds, memberChannels, revealedHuddleChannelIds],
  );
  const archivedHuddleChannels = React.useMemo(
    () =>
      sortArchivedHuddleChannels(
        memberChannels.filter((channel) =>
          isArchivedHuddleSidebarChannel(
            channel,
            huddleBackingChannelIds,
            revealedHuddleChannelIds,
            activeHuddleChannelId,
          ),
        ),
      ),
    [
      activeHuddleChannelId,
      huddleBackingChannelIds,
      memberChannels,
      revealedHuddleChannelIds,
    ],
  );
  const unreadTrackingChannels = React.useMemo(
    () =>
      archivedHuddleChannels.length === 0
        ? sidebarChannels
        : [...sidebarChannels, ...archivedHuddleChannels],
    [archivedHuddleChannels, sidebarChannels],
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
    enabled: !isHuddleRoom,
    goChannel,
    goHome,
    notificationSettings: notificationSettings.settings,
    openSearchHit,
    pubkey: identityQuery.data?.pubkey,
    silentChannelIds: huddleBackingChannelIds,
  });
  const {
    followedRootIds,
    isFollowing: isFollowingThread,
    followThread,
    unfollowThread,
  } = useThreadFollows(identityQuery.data?.pubkey);
  const { starredThreadIds } = useThreadStars(identityQuery.data?.pubkey);
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
    isReadStateReady,
    setContextParentResolver,
    participatedRootIds,
    authoredRootIds,
    recordThreadInteraction,
    threadActivityItems,
    mutedRootIds,
    muteThread,
    unmuteThread,
  } = useUnreadChannels(
    isHuddleRoom ? EMPTY_CHANNELS : unreadTrackingChannels,
    isHuddleRoom ? null : activeChannel,
    {
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
      starredRootIds: starredThreadIds,
    },
  );

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
  // Inbox one-to-one with notifications (fetched DMs/thread replies) + relay push prefs.
  const inboxActivityFeedItems = useInboxPushPairing({ pubkey: identityQuery.data?.pubkey, channels, mentions: homeFeedQuery.data?.feed.mentions, liveItems: threadActivityFeedItems,
    settings: notificationSettings.settings, followedRootIds, starredRootIds: starredThreadIds, mutedChannelIds, mutedRootIds, enabled: !isHuddleRoom });
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

  const { homeBadgeCountExcludingHighPriority } = useHomeFeedNotificationState(
    homeFeedQuery.data,
    identityQuery.data?.pubkey,
    notificationSettings.settings,
    notificationSettings.setDesktopEnabled,
    !isHuddleRoom,
    selectedView === "home" && !settingsOpen,
    getChannelReadAt,
    readStateVersion,
    highPriorityUnreadChannelIds,
    feedProfilesQuery.data?.profiles,
    mutedChannelIds,
    feedItemState.unreadSet,
    inboxActivityFeedItems,
    getThreadReadAt,
    getMessageReadAt,
    channels,
    huddleBackingChannelIds,
  );
  const isNotifiedForThread = useIsNotifiedForThread(mutedRootIds, [
    followedRootIds,
    starredThreadIds,
    participatedRootIds,
    authoredRootIds,
  ]);

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
    (hit: SearchHit, query: string) => {
      void openSearchHit(hit, { query });
    },
    [openSearchHit],
  );
  useAppShellLifecycleEffects({
    desktopBadgeEnabled: !isHuddleRoom,
    homeBadgeCountExcludingHighPriority,
    topLevelUnreadChannelIds,
    unreadChannelNotificationCount,
  });
  // Dispatch `buzz://` deep links only from the main window; the companion is dedicated to its active Huddle route.
  useAppDeepLinks(!isHuddleRoom);
  const handleOpenCreateChannel = React.useCallback(
    () => setIsCreateChannelOpen(true),
    [],
  );
  useAppShellKeyboardShortcuts({
    activeChannelId: selectedView === "channel" ? selectedChannelId : null,
    canSearchCurrentChannel:
      selectedView === "channel" && Boolean(activeChannel),
    disabled: settingsOpen || isHuddleRoom,
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
    open: isHuddleRoom ? undefined : settingsOpen,
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
      <RemoteBrowserIndicator />
      {!hideAppChrome ? (
        <AppShellTrayMenu
          channels={channels}
          goChannel={goChannel}
          openCreateChannel={handleOpenCreateChannel}
        />
      ) : null}
      <ChannelNavigationProvider channels={channels}>
        <AppShellProvider
          value={{
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
            isReadStateReady,
            setContextParentResolver,
            followThread: handleFollowThread,
            unfollowThread: handleUnfollowThread,
            isFollowingThread,
            isNotifiedForThread,
            recordThreadInteraction,
            isThreadMuted: (rootId) => mutedRootIds.has(rootId),
            threadActivityItems,
            threadActivityFeedItems: inboxActivityFeedItems,
            locallyUnreadFeedItems,
            unreadThreadFeedItems,
            unreadThreadChannelIds,
            unreadChannelIds,
            unreadChannelCounts,
            dmChannelIds,
            topLevelUnreadChannelIds,
            hasSidebarUnreadProjections: true,
            feedItemState,
            onOpenSettings: handleOpenSettings,
          }}
        >
          <AppHuddleShell
            currentPubkey={identityQuery.data?.pubkey}
            isCompanionOpen={isHuddleCompanionOpen}
            isDrawerOpen={isHuddleDrawerOpen}
            isRoom={isHuddleRoom}
            presentation={huddlePresentation}
            onCompanionOpen={handleHuddleCompanionOpen}
            onHuddleStartPendingChange={handleHuddleStartPendingChange}
            onHuddleStarted={handleHuddleStarted}
            onShowHuddleInMainApp={showHuddleInMainApp}
            onViewHuddleChannel={viewHuddleChannel}
            onVisibilityChange={handleHuddleVisibilityChange}
          >
            {hasCommunityRail && !hideAppChrome ? (
              <CommunityRail
                activeCommunityId={communitiesHook.activeCommunity?.id ?? null}
                onAddCommunity={addCommunityDialog.openDialog}
                onReorderCommunities={communitiesHook.reorderCommunities}
                onSwitchCommunity={handleSwitchCommunity}
                onUpdateCommunity={communitiesHook.updateCommunity}
                communities={communitiesHook.communities}
              />
            ) : null}
            <SidebarProvider
              className="relative z-10 min-h-0 min-w-0 flex-1 flex-col overflow-visible"
              data-testid="app-sidebar-layer"
            >
              <SurfaceTabsProvider
                communityId={communitiesHook.activeCommunity?.id ?? ""}
              >
                <AppProfilePanelProvider>
                  <AppWorkflowEditorOverlayProvider>
                    {isPhone && !hideAppChrome ? (
                      <PhoneNavRow
                        canGoBack={canGoBack}
                        canGoForward={canGoForward}
                        onGoBack={handlePhoneNavBack}
                        onGoForward={() => handlePhoneNavHistory(1)}
                        onHome={handlePhoneNavHome}
                      />
                    ) : null}
                    {!settingsOpen && !hideAppChrome ? (
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
                      <div className="relative flex min-h-0 flex-1 overflow-visible">
                        {!hideAppChrome && (!isPhone || phoneOverlay === "channels") ? (
                          <PhoneFullscreen phone={isPhone} testId="phone-channels-screen">
                          <AppSidebar
                            activeCommunity={communitiesHook.activeCommunity}
                            archivedHuddleChannels={archivedHuddleChannels}
                            channels={sidebarChannels}
                            currentPubkey={identityQuery.data?.pubkey}
                            errorMessage={channelsErrorMessage}
                            fallbackDisplayName={
                              identityQuery.data?.displayName
                            }
                            addCommunityPrefill={addCommunityDialog.prefill}
                            isAddCommunityOpen={addCommunityDialog.open}
                            relayConnectionCard={relayConnectionCard}
                            isCreatingChannel={createChannelMutation.isPending}
                            isCreatingForum={createForumMutation.isPending}
                            isLoading={channelsQuery.isLoading}
                            isCreateChannelOpen={isCreateChannelOpen}
                            isHuddleCompanionOpen={isHuddleCompanionOpen}
                            isPresencePending={presenceSession.isPending}
                            onAddCommunity={(community) => {
                              const id = communitiesHook.addCommunity({
                                ...community,
                                pubkey:
                                  community.pubkey ??
                                  identityQuery.data?.pubkey,
                              });
                              handleSwitchCommunity(id);
                            }}
                            onAddCommunityOpenChange={
                              addCommunityDialog.onOpenChange
                            }
                            onNewMessage={goNewMessage}
                            onBackgroundClick={requestFocusedThreadClose}
                            onCreateChannelOpenChange={setIsCreateChannelOpen}
                            onOpenAddCommunity={addCommunityDialog.openDialog}
                            onSendFeedback={() => setIsSendFeedbackOpen(true)}
                            onUpdateCommunity={communitiesHook.updateCommunity}
                            onLeaveCommunity={handleLeaveCommunity}
                            onRemoveCommunityFromDevice={removeFromDevice}
                            onSwitchCommunity={handleSwitchCommunity}
                            onCreateAgent={() => requestOpenCreateAgent()}
                            selfPresenceStatus={presenceSession.currentStatus}
                            communities={communitiesHook.communities}
                            onCreateChannel={handleCreateChannel}
                            onCreateForum={handleCreateForum}
                            onHideDm={handleHideDm}
                            onHuddleEnded={handleHuddleEnded}
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
                            onSelectBrowsers={() => void goBrowsers()}
                            onSelectBots={() => void goBots()}
                            onSelectPinnedSite={(pinId) =>
                              void goPinnedSite(pinId)
                            }
                            onSelectChannel={(...args: Parameters<typeof handleSidebarChannelSelect>) => {
                              setPhoneOverlay(null);
                              return handleSidebarChannelSelect(...args);
                            }}
                            onOpenSearchResult={handleOpenSearchResult}
                            searchChannels={channels}
                            searchFocusRequests={[
                              searchFocusRequest,
                              scopeSearchFocusRequest,
                            ]}
                            onSelectHome={() => void goHome()}
                            onSelectProjects={() => void goProjects()}
                            onSelectPulse={() => void goPulse()}
                            onSelectSettings={handleOpenSettings}
                            onSelectWorkflows={() => void goWorkflows()}
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
                            projectsOverviewActive={
                              location.pathname === "/projects"
                            }
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
                            selectedPinId={selectedPinId}
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
                          </PhoneFullscreen>
                        ) : null}
                        {isPhone && phoneOverlay === "me" && !hideAppChrome ? (
                          <div className="phone-fullscreen">
                            <PhoneMeScreen
                              avatarUrl={profileQuery.data?.avatarUrl}
                              displayName={
                                profileQuery.data?.displayName ||
                                identityQuery.data?.displayName ||
                                "Me"
                              }
                              onAgents={() => void goAgents()}
                              onBots={() => void goBots()}
                              onBrowsers={() => void goBrowsers()}
                              onProjects={() => void goProjects()}
                              onSettings={handleOpenSettings}
                              onWorkflows={() => void goWorkflows()}
                              pubkey={identityQuery.data?.pubkey}
                            />
                          </div>
                        ) : null}
                        {isPhone && phoneOverlay === "search" && !hideAppChrome ? (
                          <div className="phone-fullscreen phone-search-screen" data-testid="phone-search-screen">
                            <div className="phone-settings-header">
                              <h1 className="px-3 text-base font-semibold">Search</h1>
                            </div>
                            <div className="px-3 pt-2">
                              <TopbarSearch
                                channels={channels}
                                currentPubkey={identityQuery.data?.pubkey}
                                onOpenChannel={(channelId) => {
                                  setPhoneOverlay(null);
                                  void goChannel(channelId);
                                }}
                                onOpenResult={(hit, query) => {
                                  setPhoneOverlay(null);
                                  handleOpenSearchResult(hit, query);
                                }}
                                phoneScreen
                                suggestionChannels={channels}
                              />
                            </div>
                          </div>
                        ) : null}
                        {isPhone && !hideAppChrome ? (
                          <PhoneTabSwitcher
                            channels={channels}
                            onOpenBuzzTerm={() => {
                              setPhoneOverlay(null);
                              void navigate({ to: "/buzz-term" });
                            }}
                            terminalActive={
                              phoneOverlay === null &&
                              location.pathname.startsWith("/buzz-term")
                            }
                          />
                        ) : null}
                        {showPhoneTabBar && !hideAppChrome ? (
                          <PhoneTabBar
                            active={phoneTab}
                            badges={{
                              channels: unreadChannelNotificationCount,
                            }}
                            me={{
                              avatarUrl: profileQuery.data?.avatarUrl,
                              displayName:
                                profileQuery.data?.displayName ||
                                identityQuery.data?.displayName ||
                                "Me",
                              pubkey: identityQuery.data?.pubkey,
                            }}
                            onSelect={(tab: PhoneTab) => {
                              if (tab === "channels" || tab === "me" || tab === "search") {
                                setPhoneOverlay(tab);
                                return;
                              }
                              setPhoneOverlay(null);
                              if (tab === "inbox") void goHome();
                              else void navigate({ to: "/buzz-term" });
                            }}
                          />
                        ) : null}
                        <TerminalContextOverrideProvider
                          onChange={setTerminalContextOverride}
                        >
                          <AppShellChannelSurface
                            hasCommunityRail={hasCommunityRail}
                            isHuddleRoom={isHuddleRoom}
                            isHuddleRoomStarting={isHuddleRoomStarting}
                            mainInsetRef={mainInsetRef}
                            terminal={
                              <TerminalBootstrap
                                {...effectiveTerminalContext}
                              />
                            }
                          >
                            <Outlet />
                          </AppShellChannelSurface>
                        </TerminalContextOverrideProvider>
                        {!hideAppChrome ? (
                          <RelayConnectionOverlay
                            card={relayConnectionCard}
                            errorMessage={channelsErrorMessage}
                            hasCommunityRail={hasCommunityRail}
                            isHuddleDrawerOpen={isHuddleDrawerOpen}
                          />
                        ) : null}
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
                      relayUrl={communitiesHook.activeCommunity?.relayUrl}
                    />
                    <SendFeedbackController
                      onOpenChange={setIsSendFeedbackOpen}
                      open={isSendFeedbackOpen}
                    />
                    {!hideAppChrome ? <ProtectedGlobalOverlay /> : null}
                  </AppWorkflowEditorOverlayProvider>
                </AppProfilePanelProvider>
              </SurfaceTabsProvider>
            </SidebarProvider>
          </AppHuddleShell>
        </AppShellProvider>
      </ChannelNavigationProvider>
    </PreventSleepProvider>
  );
}

/** Phone: hosts a tab's list screen full screen above the tab bar. Elsewhere renders children unwrapped (a Fragment), so
 * the desktop sidebar keeps its sibling relationship with the inset (Tailwind `peer-*`). */
function PhoneFullscreen({
  phone,
  testId,
  children,
}: {
  phone: boolean;
  testId?: string;
  children: React.ReactNode;
}) {
  return phone ? (
    <div className="phone-fullscreen" data-testid={testId}>
      {children}
    </div>
  ) : (
    <>{children}</>
  );
}

type PhoneNavEntry = { key: string; sig: string; list: boolean };
const PHONE_NAV_KEY = "buzz-phone-nav-stack.v1";
function readPhoneNavMap(): Record<number, PhoneNavEntry> {
  try {
    return JSON.parse(sessionStorage.getItem(PHONE_NAV_KEY) || "{}");
  } catch {
    return {};
  }
}
function writePhoneNavMap(map: Record<number, PhoneNavEntry>): void {
  try {
    sessionStorage.setItem(PHONE_NAV_KEY, JSON.stringify(map));
  } catch {
    /* storage denied */
  }
}

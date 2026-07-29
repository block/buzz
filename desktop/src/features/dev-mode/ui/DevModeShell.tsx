import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";

import { useChannelsQuery } from "@/features/channels/hooks";
import {
  type ChannelRef,
  DevChannelRefsProvider,
} from "@/features/dev-mode/lib/channelRefs";
import {
  groupSessionChannels,
  usePinnedChannels,
} from "@/features/dev-mode/lib/pinnedChannels";
import {
  loadLastComposerModeKey,
  storeLastComposerModeKey,
} from "@/features/dev-mode/lib/composerModePreference";
import type { MentionRecord } from "@/features/dev-mode/lib/mentionRecords";
import {
  aggregateLastActivity,
  aggregateUnreadMains,
  indexSubChannels,
} from "@/features/dev-mode/lib/subChannels";
import { selectRootEvents } from "@/features/dev-mode/lib/transcriptRoots";
import { useShellFocusGuards } from "@/features/dev-mode/lib/useShellFocusGuards";
import { useUnreadRouting } from "@/features/dev-mode/lib/useUnreadRouting";
import {
  devComposerModeLabel,
  useDevComposerModes,
  type DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";
import { useDevSessionActions } from "@/features/dev-mode/lib/useDevSessionActions";
import { useDevModeShortcuts } from "@/features/dev-mode/lib/useDevModeShortcuts";
import { useNavigatorWidth } from "@/features/dev-mode/lib/useNavigatorWidth";
import { DevChannelMembers } from "@/features/dev-mode/ui/DevChannelMembers";
import { DevChannelNavigator } from "@/features/dev-mode/ui/DevChannelNavigator";
import { DevChannelTabs } from "@/features/dev-mode/ui/DevChannelTabs";
import { DevCommandPalette } from "@/features/dev-mode/ui/DevCommandPalette";
import { DevPromptComposer } from "@/features/dev-mode/ui/DevPromptComposer";
import { DevShortcutsOverlay } from "@/features/dev-mode/ui/DevShortcutsOverlay";
import { DevSplitPane } from "@/features/dev-mode/ui/DevSplitPane";
import { DevThreadPanel } from "@/features/dev-mode/ui/DevThreadPanel";
import { DevTranscript } from "@/features/dev-mode/ui/DevTranscript";
import { useChannelMessagesQuery } from "@/features/messages/hooks";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { useIdentityQuery } from "@/shared/api/hooks";
import { cn } from "@/shared/lib/cn";
import { isMacPlatform } from "@/shared/lib/platform";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useIsFullscreen } from "@/shared/lib/useIsFullscreen";

/**
 * Stable identity for the cycled mode. Selection is keyed rather than
 * indexed so agent list refreshes cannot silently retarget the next prompt
 * at a different agent; a vanished agent falls back to the default agent.
 */
function devComposerModeKey(mode: DevComposerMode): string {
  return mode.kind === "chat" ? "chat" : normalizePubkey(mode.target.pubkey);
}

/**
 * Keyboard model:
 *
 * - `fresh` — just the composer. Enter spawns a session channel; ↑ slides
 *   the channel navigator out from the left.
 * - `navigator` — ↑/↓ preview channels (transcript shows behind), Enter
 *   opens the highlighted channel, Escape returns to fresh.
 * - `channel` — Enter sends; empty ↑/↓ walk prompt cards; Enter on a card
 *   opens the split-screen side chat; Escape unwinds side chat → card →
 *   navigator.
 *
 * ⌘K (anywhere) or `/` (empty composer) opens the command palette.
 */
type ShellView = "fresh" | "navigator" | "channel";

export function DevModeShell({
  unreadChannelIds,
  topLevelUnreadChannelIds,
  hasCommunityRail = false,
}: {
  /** Channels with anything unread, including relevant thread replies. */
  unreadChannelIds: ReadonlySet<string>;
  /** Channels with unread channel-level posts only. */
  topLevelUnreadChannelIds: ReadonlySet<string>;
  /** The community rail sits under the macOS traffic lights when present. */
  hasCommunityRail?: boolean;
}) {
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const isFullscreen = useIsFullscreen();
  const modes = useDevComposerModes();
  const { createSessionChannel, createSubChannel, sendToSession } =
    useDevSessionActions(identityQuery.data);

  const [view, setView] = React.useState<ShellView>("fresh");
  const [input, setInput] = React.useState("");
  const [modeKey, setModeKey] = React.useState<string | null>(
    loadLastComposerModeKey,
  );
  const [activeSessionId, setActiveSessionId] = React.useState<string | null>(
    null,
  );
  const [navigatorId, setNavigatorId] = React.useState<string | null>(null);
  const [selectedRootId, setSelectedRootId] = React.useState<string | null>(
    null,
  );
  const [threadOpen, setThreadOpen] = React.useState(false);
  const [activePane, setActivePane] = React.useState<"main" | "thread">("main");
  // When set, the composer's next Enter spawns a sub-channel of this main
  // channel instead of posting to the open channel.
  const [subDraftParentId, setSubDraftParentId] = React.useState<string | null>(
    null,
  );
  const [paletteOpen, setPaletteOpen] = React.useState(false);
  const [paletteInitialMode, setPaletteInitialMode] = React.useState<
    "root" | "members"
  >("root");
  const [shortcutsOpen, setShortcutsOpen] = React.useState(false);
  const [focusSignal, setFocusSignal] = React.useState(0);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const focusComposer = React.useCallback(() => {
    setFocusSignal((current) => current + 1);
  }, []);

  // While a prompt card is selected the caret leaves the message box — the
  // shell owns ↑/↓/Enter/Escape via a window listener until the selection
  // clears (Escape, ↓ past the newest card, or a click on the box).
  const cardSelectionActive =
    view === "channel" && selectedRootId !== null && !threadOpen;

  // The composer remembers the last target the user talked to (persisted
  // across launches). Before any selection exists — or when the remembered
  // agent vanishes — it falls back to the default: the first managed (local)
  // agent, else the first agent; plain chat only when no agents exist.
  const defaultModeIndex = React.useMemo(() => {
    const managedIndex = modes.findIndex(
      (candidate) =>
        candidate.kind === "agent" && candidate.target.source === "managed",
    );
    if (managedIndex !== -1) return managedIndex;
    const agentIndex = modes.findIndex(
      (candidate) => candidate.kind === "agent",
    );
    return agentIndex === -1 ? 0 : agentIndex;
  }, [modes]);

  const foundModeIndex =
    modeKey === null
      ? -1
      : modes.findIndex(
          (candidate) => devComposerModeKey(candidate) === modeKey,
        );
  const modeIndex = foundModeIndex === -1 ? defaultModeIndex : foundModeIndex;
  const mode = modes[modeIndex];

  const sessions = React.useMemo(
    () =>
      (channelsQuery.data ?? []).filter(
        (channel) =>
          channel.channelType === "stream" &&
          channel.isMember &&
          channel.archivedAt === null,
      ),
    [channelsQuery.data],
  );

  // Open channels the user hasn't joined: the palette searches these and
  // joins on enter, but they stay out of the left navigator until joined.
  const discoverableChannels = React.useMemo(
    () =>
      (channelsQuery.data ?? []).filter(
        (channel) =>
          channel.channelType === "stream" &&
          !channel.isMember &&
          channel.visibility === "open" &&
          channel.archivedAt === null,
      ),
    [channelsQuery.data],
  );

  // `#channel` references: composers autocomplete these names and message
  // rows render matching tokens as clickable links to the channel.
  const channelRefs = React.useMemo<ChannelRef[]>(
    () => sessions.map((channel) => ({ id: channel.id, name: channel.name })),
    [sessions],
  );

  // `parent--sub` channels pair with their parents: only mains render in
  // the left list; subs surface as tabs inside their parent.
  const subIndex = React.useMemo(() => indexSubChannels(sessions), [sessions]);

  // The left list orders mains by their whole family's latest activity, so
  // a busy sub-channel floats its parent.
  const listChannels = React.useMemo(() => {
    const overrides = aggregateLastActivity(subIndex);
    if (overrides.size === 0) return subIndex.mains;
    return subIndex.mains.map((channel) => {
      const latest = overrides.get(channel.id);
      return latest && latest > (channel.lastMessageAt ?? "")
        ? { ...channel, lastMessageAt: latest }
        : channel;
    });
  }, [subIndex]);

  const navigatorUnreadIds = React.useMemo(
    () => aggregateUnreadMains(subIndex, unreadChannelIds),
    [subIndex, unreadChannelIds],
  );

  const pinnedIds = usePinnedChannels();
  // Pinned chats on top, everything else below — each newest-first; `flat`
  // matches the navigator's render order so ↑/↓ walk what is on screen.
  const { groups: channelGroups, flat: orderedChannels } = React.useMemo(
    () => groupSessionChannels(listChannels, pinnedIds),
    [pinnedIds, listChannels],
  );

  const findChannel = React.useCallback(
    (channelId: string | null) =>
      (channelsQuery.data ?? []).find((channel) => channel.id === channelId) ??
      null,
    [channelsQuery.data],
  );

  const activeChannel =
    view === "channel" ? findChannel(activeSessionId) : null;
  const previewChannel = view === "navigator" ? findChannel(navigatorId) : null;
  const topBarChannel = activeChannel ?? previewChannel;
  // A stored id whose channel vanished (or is still propagating) renders as
  // the fresh-session state; navigation starts from what is actually shown.
  const effectiveSessionId = activeChannel?.id ?? null;

  // Logical selection: the open channel's main. When a sub tab is active,
  // the left list keeps highlighting the parent and ⌥↑↓/Escape navigate by
  // parent; the transcript and composer stay on the physical channel.
  const activeMainId = activeChannel
    ? (subIndex.parentIdByChildId.get(activeChannel.id) ?? activeChannel.id)
    : null;
  const activeMainChannel = activeMainId ? findChannel(activeMainId) : null;
  const activeSubChannels = React.useMemo(
    () =>
      activeMainId ? (subIndex.subsByParentId.get(activeMainId) ?? []) : [],
    [activeMainId, subIndex],
  );
  const subDraftActive =
    view === "channel" &&
    subDraftParentId !== null &&
    subDraftParentId === activeMainId;

  // Shares the transcript's query cache — used only for card navigation.
  const messagesQuery = useChannelMessagesQuery(activeChannel);
  const roots = React.useMemo(
    () => selectRootEvents(messagesQuery.data),
    [messagesQuery.data],
  );
  const selectedRoot = roots.find((root) => root.id === selectedRootId) ?? null;

  // Viewing an open channel marks its channel-level posts read (same passive
  // NIP-RS path the standard channel screen uses). topLevelOnly keeps thread
  // replies out of the marker — thread unread clears through what is actually
  // seen: the inline first reply (transcript) and the side chat (panel).
  const { markChannelRead } = useAppShell();
  const latestRootAt =
    roots.length > 0 ? roots[roots.length - 1].created_at : null;
  const activeChannelIdForRead = activeChannel?.isMember
    ? activeChannel.id
    : null;
  React.useEffect(() => {
    if (!activeChannelIdForRead) return;
    markChannelRead(
      activeChannelIdForRead,
      latestRootAt === null
        ? null
        : new Date(latestRootAt * 1_000).toISOString(),
      { topLevelOnly: true },
    );
  }, [activeChannelIdForRead, latestRootAt, markChannelRead]);

  // Card selection and the side chat belong to one channel's transcript.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — selection resets only on channel switch
  React.useEffect(() => {
    setSelectedRootId(null);
    setThreadOpen(false);
    setActivePane("main");
    setSubDraftParentId(null);
  }, [effectiveSessionId]);

  // Window refocus restores the last text input; dead-space clicks never
  // blur it (see useShellFocusGuards).
  const { handleFocusCapture, handleShellMouseDown, handleShellMouseUp } =
    useShellFocusGuards({ cardSelectionActive, focusComposer });

  // Lifted here (not inside the navigator) so the top bar's columns track
  // the navigator width live while the divider is dragged.
  const navigatorWidthControls = useNavigatorWidth();

  const closePalette = React.useCallback(() => {
    setPaletteOpen(false);
    focusComposer();
  }, [focusComposer]);

  const openPalette = React.useCallback((mode: "root" | "members" = "root") => {
    setPaletteInitialMode(mode);
    setPaletteOpen(true);
  }, []);

  const closeShortcuts = React.useCallback(() => {
    setShortcutsOpen(false);
    focusComposer();
  }, [focusComposer]);

  const openChannel = React.useCallback(
    (channelId: string) => {
      setActiveSessionId(channelId);
      // The left list only shows mains — highlight the family's parent when
      // a sub tab is opened directly (palette, #ref link, tab click).
      setNavigatorId(subIndex.parentIdByChildId.get(channelId) ?? channelId);
      setView("channel");
      focusComposer();
    },
    [focusComposer, subIndex],
  );

  const handleOpenThread = React.useCallback((rootId: string) => {
    setSelectedRootId(rootId);
    setThreadOpen(true);
    setActivePane("thread");
  }, []);

  const openChannelAtUnread = useUnreadRouting({
    subIndex,
    unreadChannelIds,
    topLevelUnreadChannelIds,
    activeChannel,
    roots,
    openChannel,
    openThread: handleOpenThread,
  });

  // "+ tab" (tab strip, palette, or ⌘⇧T): the composer's next Enter spawns
  // a new tab (sub-channel) of the open main instead of posting to the
  // channel.
  const startSubChannelDraft = React.useCallback(() => {
    if (!activeMainId) return;
    setSubDraftParentId(activeMainId);
    setSelectedRootId(null);
    setThreadOpen(false);
    setActivePane("main");
    focusComposer();
  }, [activeMainId, focusComposer]);

  const goToFresh = React.useCallback(() => {
    setView("fresh");
    setActiveSessionId(null);
    setThreadOpen(false);
    setSelectedRootId(null);
    setSubDraftParentId(null);
    focusComposer();
  }, [focusComposer]);

  // Leaving/archiving a chat lands on the most recently active non-pinned
  // chat (the departed channel may still be in the cached list, so exclude
  // it); with nowhere to go, fall back to the fresh composer.
  const handleChannelLeft = React.useCallback(
    (leftChannelId: string) => {
      // Leaving a sub tab returns to its parent's main tab.
      const parentId = subIndex.parentIdByChildId.get(leftChannelId);
      if (parentId) {
        openChannel(parentId);
        return;
      }
      const next = channelGroups
        .find((group) => !group.pinned)
        ?.channels.find((channel) => channel.id !== leftChannelId);
      if (next) {
        openChannel(next.id);
      } else {
        goToFresh();
      }
    },
    [channelGroups, goToFresh, openChannel, subIndex],
  );

  // ⌘T's draft side chat: the pane opens with no thread yet; its first send
  // posts a new message to the channel and attaches the pane to that thread.
  const draftSideChat = React.useCallback(() => {
    setSelectedRootId(null);
    setThreadOpen(true);
    setActivePane("thread");
  }, []);

  const togglePalette = React.useCallback(() => {
    setPaletteInitialMode("root");
    setPaletteOpen((current) => !current);
  }, []);

  useDevModeShortcuts({
    view,
    activeChannel,
    activeMainChannel,
    activeSubChannels,
    onTogglePalette: togglePalette,
    onNewSession: goToFresh,
    onDraftSideChat:
      view === "channel" && activeSessionId ? draftSideChat : null,
    onDraftTab:
      view === "channel" && activeMainId ? startSubChannelDraft : null,
    onOpenChannel: openChannel,
  });

  const handleCycleMode = React.useCallback(
    (direction: 1 | -1) => {
      if (modes.length === 0) return;
      const nextIndex = (modeIndex + direction + modes.length) % modes.length;
      const nextKey = devComposerModeKey(modes[nextIndex]);
      setModeKey(nextKey);
      storeLastComposerModeKey(nextKey);
    },
    [modeIndex, modes],
  );

  const navigateChannels = React.useCallback(
    (direction: 1 | -1) => {
      if (orderedChannels.length === 0) return;
      const currentIndex = orderedChannels.findIndex(
        (session) => session.id === navigatorId,
      );
      if (currentIndex === -1) {
        setNavigatorId(orderedChannels[orderedChannels.length - 1].id);
        return;
      }
      // ↑ walks up the visible list; ↓ back down. The navigator stays
      // highlighted at the ends — only Enter or Escape leave it.
      const nextIndex = Math.min(
        orderedChannels.length - 1,
        Math.max(0, currentIndex + direction),
      );
      setNavigatorId(orderedChannels[nextIndex].id);
    },
    [navigatorId, orderedChannels],
  );

  // ⌥↑/⌥↓ from the composer: open the previous/next channel in the visible
  // list directly — focus stays in the box the whole time.
  const stepChannel = React.useCallback(
    (direction: 1 | -1) => {
      if (orderedChannels.length === 0) return;
      const referenceId = view === "channel" ? activeMainId : navigatorId;
      const currentIndex = orderedChannels.findIndex(
        (session) => session.id === referenceId,
      );
      if (currentIndex === -1) {
        // Nothing open — ⌥↑ enters the list at the bottom (nearest channel).
        if (direction === -1) {
          openChannel(orderedChannels[orderedChannels.length - 1].id);
        }
        return;
      }
      const nextIndex = Math.min(
        orderedChannels.length - 1,
        Math.max(0, currentIndex + direction),
      );
      if (nextIndex === currentIndex) return;
      openChannel(orderedChannels[nextIndex].id);
    },
    [activeMainId, navigatorId, openChannel, orderedChannels, view],
  );

  const navigateCards = React.useCallback(
    (direction: 1 | -1) => {
      if (roots.length === 0) return;
      const currentIndex = roots.findIndex(
        (root) => root.id === selectedRootId,
      );
      if (currentIndex === -1) {
        // ArrowUp enters the cards at the newest prompt; ArrowDown is a no-op.
        if (direction === -1) {
          setSelectedRootId(roots[roots.length - 1].id);
        }
        return;
      }
      const nextIndex = currentIndex + direction;
      if (nextIndex >= roots.length) {
        // Past the newest card — back to plain channel input.
        setSelectedRootId(null);
        setThreadOpen(false);
        return;
      }
      setSelectedRootId(roots[Math.max(0, nextIndex)].id);
    },
    [roots, selectedRootId],
  );

  const handleNavigate = React.useCallback(
    (direction: 1 | -1) => {
      if (view === "channel") {
        navigateCards(direction);
        return;
      }
      if (view === "fresh") {
        if (direction === -1) {
          setView("navigator");
          setNavigatorId(
            orderedChannels.length > 0
              ? orderedChannels[orderedChannels.length - 1].id
              : null,
          );
        }
        return;
      }
      navigateChannels(direction);
    },
    [navigateChannels, navigateCards, orderedChannels, view],
  );

  const handleEscape = React.useCallback(() => {
    if (view === "channel") {
      if (subDraftActive) {
        setSubDraftParentId(null);
        focusComposer();
        return;
      }
      if (threadOpen) {
        setThreadOpen(false);
        setActivePane("main");
        focusComposer();
        return;
      }
      if (selectedRootId) {
        setSelectedRootId(null);
        return;
      }
      // Back out to the navigator with the current channel's main
      // highlighted (subs have no row of their own).
      setNavigatorId(activeMainId);
      setActiveSessionId(null);
      setView("navigator");
      return;
    }
    if (view === "navigator") {
      goToFresh();
    }
  }, [
    activeMainId,
    focusComposer,
    goToFresh,
    selectedRootId,
    subDraftActive,
    threadOpen,
    view,
  ]);

  const handleSwitchPane = React.useCallback(
    (pane: "main" | "thread") => {
      if (!threadOpen) return;
      setActivePane(pane);
      if (pane === "main") {
        focusComposer();
      }
    },
    [focusComposer, threadOpen],
  );

  const handleSubmit = React.useCallback(
    (mentions: MentionRecord[] = [], media: ImetaMedia[] = []) => {
      const prompt = input.trim();
      // A media-only send is a real send inside a channel; elsewhere the
      // empty-input Enter keeps its navigation meaning (a fresh-composer
      // channel needs prompt text for naming anyway).
      const mediaOnlySend =
        !prompt && media.length > 0 && view === "channel" && activeChannel;
      if (!prompt && !mediaOnlySend) {
        if (view === "navigator" && navigatorId) {
          openChannelAtUnread(navigatorId);
          return;
        }
        // Empty-input Enter opens the selected card's side chat.
        if (view === "channel" && selectedRootId) {
          handleOpenThread(selectedRootId);
        }
        return;
      }
      if (busy || !mode) return;

      storeLastComposerModeKey(devComposerModeKey(mode));
      setBusy(true);
      setError(null);
      setInput("");
      void (async () => {
        try {
          let channel = activeChannel;
          if (!channel) {
            channel = await createSessionChannel(prompt, mode);
            setActiveSessionId(channel.id);
            setNavigatorId(channel.id);
            setView("channel");
          } else if (subDraftActive && activeMainChannel) {
            // "+ sub" draft: spawn a sub-channel of the open main and land
            // on its tab; the prompt goes to the new sub, not the main.
            channel = await createSubChannel(activeMainChannel, prompt, mode);
            setSubDraftParentId(null);
            setActiveSessionId(channel.id);
          }
          await sendToSession(
            channel,
            prompt,
            mode,
            undefined,
            mentions,
            media,
          );
          // The conversation moved to the new prompt at the bottom.
          setSelectedRootId(null);
        } catch (submitError) {
          setError(
            submitError instanceof Error
              ? submitError.message
              : "Failed to send prompt.",
          );
          // Restore the failed prompt unless the user already typed on.
          setInput((current) => (current === "" ? prompt : current));
        } finally {
          setBusy(false);
        }
      })();
    },
    [
      activeChannel,
      activeMainChannel,
      busy,
      createSessionChannel,
      createSubChannel,
      handleOpenThread,
      input,
      mode,
      navigatorId,
      openChannelAtUnread,
      selectedRootId,
      sendToSession,
      subDraftActive,
      view,
    ],
  );

  const handleThreadSend = React.useCallback(
    async (prompt: string, mentions: MentionRecord[], media: ImetaMedia[]) => {
      if (!activeChannel || !mode) {
        throw new Error("Thread is no longer available.");
      }
      storeLastComposerModeKey(devComposerModeKey(mode));
      if (selectedRoot) {
        await sendToSession(
          activeChannel,
          prompt,
          mode,
          selectedRoot.id,
          mentions,
          media,
        );
        return;
      }
      // Draft side chat (⌘T): the first send posts a root message to the
      // channel exactly like the main composer, then the pane attaches to
      // that new thread.
      const newRoot = await sendToSession(
        activeChannel,
        prompt,
        mode,
        undefined,
        mentions,
        media,
      );
      setSelectedRootId(newRoot.id);
    },
    [activeChannel, mode, selectedRoot, sendToSession],
  );

  const placeholder = subDraftActive
    ? `Prompt spawns a new tab in # ${activeMainChannel?.name ?? ""}…`
    : activeChannel
      ? mode?.kind === "agent"
        ? `Message # ${activeChannel.name} and put ${devComposerModeLabel(mode)} to work…`
        : `Message # ${activeChannel.name}…`
      : mode?.kind === "agent"
        ? `Prompt ${devComposerModeLabel(mode)} — spawns a new channel where it works…`
        : "Start a discussion — spawns a new channel for humans…";

  const composerActive =
    !paletteOpen &&
    !shortcutsOpen &&
    !(threadOpen && activePane === "thread") &&
    !cardSelectionActive;

  React.useEffect(() => {
    if (!cardSelectionActive || paletteOpen || shortcutsOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      // A focused input owns its own keys (e.g. a click landed in one).
      if (
        event.target instanceof HTMLElement &&
        event.target.matches("textarea, input, [contenteditable='true']")
      ) {
        return;
      }
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        event.preventDefault();
        navigateCards(event.key === "ArrowUp" ? -1 : 1);
      } else if (event.key === "Enter") {
        event.preventDefault();
        if (selectedRootId) handleOpenThread(selectedRootId);
      } else if (event.key === "Escape") {
        event.preventDefault();
        handleEscape();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [
    cardSelectionActive,
    handleEscape,
    handleOpenThread,
    navigateCards,
    paletteOpen,
    selectedRootId,
    shortcutsOpen,
  ]);

  const transcriptFor = (
    channel: NonNullable<typeof activeChannel>,
    { markRead = false } = {},
  ) => (
    <DevTranscript
      channel={channel}
      currentPubkey={identityQuery.data?.pubkey ?? null}
      markRead={markRead}
      onOpenThread={handleOpenThread}
      selectedRootId={view === "channel" ? selectedRootId : null}
    />
  );

  const sideChatOpen = Boolean(
    view === "channel" && activeChannel && threadOpen && mode,
  );

  const composer = mode ? (
    <DevPromptComposer
      active={composerActive}
      busy={busy}
      channelId={activeChannel?.id ?? null}
      draftLabel={
        subDraftActive ? `new tab in # ${activeMainChannel?.name ?? ""}` : null
      }
      focusSignal={focusSignal}
      mode={mode}
      onChange={setInput}
      onCycleMode={handleCycleMode}
      onEscape={handleEscape}
      onNavigate={handleNavigate}
      onOpenPalette={() => openPalette()}
      onOpenShortcuts={() => setShortcutsOpen(true)}
      onStepChannel={stepChannel}
      onReactivate={() => {
        if (cardSelectionActive) setSelectedRootId(null);
      }}
      onSubmit={handleSubmit}
      onSwitchPane={handleSwitchPane}
      placeholder={placeholder}
      selfPubkey={identityQuery.data?.pubkey ?? null}
      value={input}
    />
  ) : null;

  const errorBar = error ? (
    <div className="border-t border-destructive/40 bg-destructive/10 px-4 py-1.5 font-mono text-xs text-destructive">
      {error}
    </div>
  ) : null;

  // Fixed px clearance: the native macOS traffic lights overlay this strip
  // and ignore the app's text zoom, so rem-based padding would slide the
  // title under them. The 56px community rail absorbs most of the lights'
  // ~88px footprint, leaving ~32px protruding into the shell.
  const macChrome = isMacPlatform() && !isFullscreen;
  const titleClearance = macChrome
    ? hasCommunityRail
      ? "pl-[32px]"
      : "pl-[88px]"
    : "pl-4";

  return (
    <DevChannelRefsProvider channels={channelRefs} openChannel={openChannel}>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: handlers only guard focus (track last input, keep dead-space clicks from blurring it) — the div is not interactive */}
      <div
        className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-background"
        data-testid="dev-mode-shell"
        onFocusCapture={handleFocusCapture}
        onMouseDown={handleShellMouseDown}
        onMouseUp={handleShellMouseUp}
      >
        {/* Two columns sharing the navigator's live width so "buzz ·
            developer mode" sits over the channel list and the channel
            name/members sit over the transcript, even mid-drag. */}
        <div
          className="flex h-[40px] shrink-0 cursor-default select-none items-center border-b border-border/60 font-mono text-xs text-muted-foreground"
          data-tauri-drag-region
        >
          <div
            className={cn("flex h-full shrink-0 items-center", titleClearance)}
            data-tauri-drag-region
            style={{ width: navigatorWidthControls.width }}
          >
            <span
              className={cn(
                "pointer-events-none truncate",
                macChrome && "translate-y-[3px]",
              )}
            >
              buzz · developer mode
            </span>
          </div>
          <div
            className="flex h-full min-w-0 flex-1 items-center justify-between gap-3 pr-4 pl-4"
            data-tauri-drag-region
          >
            <span
              className={cn(
                "pointer-events-none min-w-0 truncate whitespace-nowrap text-foreground",
                macChrome && "translate-y-[3px]",
              )}
              data-testid="dev-mode-topbar-channel"
            >
              {topBarChannel ? <># {topBarChannel.name}</> : null}
            </span>
            {topBarChannel ? (
              <span
                className={cn(
                  "flex min-w-0 shrink-0 items-baseline",
                  macChrome && "translate-y-[3px]",
                )}
              >
                <DevChannelMembers
                  channel={topBarChannel}
                  onShowMembers={() => openPalette("members")}
                />
              </span>
            ) : null}
          </div>
        </div>

        <div className="flex min-h-0 min-w-0 flex-1">
          <DevChannelNavigator
            dimmed={view === "channel"}
            groups={channelGroups}
            highlightedId={view === "fresh" ? null : navigatorId}
            onOpen={openChannelAtUnread}
            unreadChannelIds={navigatorUnreadIds}
            widthControls={navigatorWidthControls}
          />

          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            {view === "channel" && activeChannel && activeMainChannel ? (
              <DevChannelTabs
                activeId={activeChannel.id}
                main={activeMainChannel}
                onNewSubChannel={startSubChannelDraft}
                onSelect={openChannel}
                subs={activeSubChannels}
                unreadChannelIds={unreadChannelIds}
              />
            ) : null}
            {view === "navigator" && previewChannel ? (
              <div className="pointer-events-none flex min-h-0 min-w-0 flex-1 flex-col opacity-70">
                <div className="shrink-0 border-b border-border/60 px-4 py-1 font-mono text-xs text-muted-foreground/60">
                  preview
                </div>
                {transcriptFor(previewChannel)}
              </div>
            ) : view === "channel" && activeChannel ? (
              threadOpen && mode ? (
                <DevSplitPane
                  activePane={activePane}
                  main={
                    <>
                      {transcriptFor(activeChannel, {
                        markRead: activeChannel.isMember,
                      })}
                      {composer}
                    </>
                  }
                  side={
                    <DevThreadPanel
                      active={activePane === "thread"}
                      channel={activeChannel}
                      currentPubkey={identityQuery.data?.pubkey ?? null}
                      mode={mode}
                      onClose={() => {
                        setThreadOpen(false);
                        setActivePane("main");
                        focusComposer();
                      }}
                      onCycleMode={handleCycleMode}
                      onSend={handleThreadSend}
                      onSwitchPane={handleSwitchPane}
                      root={selectedRoot}
                    />
                  }
                />
              ) : (
                transcriptFor(activeChannel, {
                  markRead: activeChannel.isMember,
                })
              )
            ) : (
              <div className="flex min-h-0 flex-1 items-center justify-center px-8 font-mono text-sm text-muted-foreground">
                <div className="max-w-lg space-y-2">
                  <div className="text-foreground">new session</div>
                  <div>
                    Type a prompt — it spawns a channel and puts the selected
                    target to work. Type ? for keyboard shortcuts.
                  </div>
                </div>
              </div>
            )}

            {/* Inside a channel the composer covers only this pane; the fresh
              and navigator states' composer below spans the full shell. */}
            {view === "channel" && !sideChatOpen ? (
              <>
                {errorBar}
                {composer}
              </>
            ) : null}
            {sideChatOpen ? errorBar : null}
          </div>
        </div>

        {view !== "channel" ? (
          <>
            {errorBar}
            {composer}
          </>
        ) : null}

        {paletteOpen ? (
          <DevCommandPalette
            activeChannel={topBarChannel}
            channels={[...sessions].reverse()}
            discoverableChannels={discoverableChannels}
            initialMode={paletteInitialMode}
            myPubkey={identityQuery.data?.pubkey ?? null}
            onChannelLeft={handleChannelLeft}
            onClose={closePalette}
            onNewSession={goToFresh}
            onShowShortcuts={() => setShortcutsOpen(true)}
            onNewSubChannel={
              view === "channel" && activeMainChannel
                ? startSubChannelDraft
                : null
            }
            onOpenChannel={openChannel}
            parentOfActive={
              topBarChannel
                ? (findChannel(
                    subIndex.parentIdByChildId.get(topBarChannel.id) ?? null,
                  ) ?? null)
                : null
            }
          />
        ) : null}

        {shortcutsOpen ? (
          <DevShortcutsOverlay onClose={closeShortcuts} />
        ) : null}
      </div>
    </DevChannelRefsProvider>
  );
}

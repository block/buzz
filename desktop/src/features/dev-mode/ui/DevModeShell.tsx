import * as React from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import { setDisplayStyle } from "@/features/dev-mode/lib/displayStylePreference";
import { selectRootEvents } from "@/features/dev-mode/lib/transcriptRoots";
import {
  devComposerModeLabel,
  useDevComposerModes,
  type DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";
import { useDevSessionActions } from "@/features/dev-mode/lib/useDevSessionActions";
import { DevChannelNavigator } from "@/features/dev-mode/ui/DevChannelNavigator";
import { DevCommandPalette } from "@/features/dev-mode/ui/DevCommandPalette";
import { DevPromptComposer } from "@/features/dev-mode/ui/DevPromptComposer";
import { DevSplitPane } from "@/features/dev-mode/ui/DevSplitPane";
import { DevThreadPanel } from "@/features/dev-mode/ui/DevThreadPanel";
import { DevTranscript } from "@/features/dev-mode/ui/DevTranscript";
import { useChannelMessagesQuery } from "@/features/messages/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Stable identity for the cycled mode. Selection is keyed rather than
 * indexed so agent list refreshes cannot silently retarget the next prompt
 * at a different agent; a vanished agent falls back to chat.
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
 * Ctrl+O (anywhere) or `/` (empty composer) opens the command palette.
 */
type ShellView = "fresh" | "navigator" | "channel";

export function DevModeShell() {
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const modes = useDevComposerModes();
  const { createSessionChannel, sendToSession } = useDevSessionActions(
    identityQuery.data,
  );

  const [view, setView] = React.useState<ShellView>("fresh");
  const [input, setInput] = React.useState("");
  const [modeKey, setModeKey] = React.useState("chat");
  const [activeSessionId, setActiveSessionId] = React.useState<string | null>(
    null,
  );
  const [navigatorId, setNavigatorId] = React.useState<string | null>(null);
  const [selectedRootId, setSelectedRootId] = React.useState<string | null>(
    null,
  );
  const [threadOpen, setThreadOpen] = React.useState(false);
  const [activePane, setActivePane] = React.useState<"main" | "thread">("main");
  const [paletteOpen, setPaletteOpen] = React.useState(false);
  const [focusSignal, setFocusSignal] = React.useState(0);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const focusComposer = React.useCallback(() => {
    setFocusSignal((current) => current + 1);
  }, []);

  const foundModeIndex = modes.findIndex(
    (candidate) => devComposerModeKey(candidate) === modeKey,
  );
  const mode = modes[foundModeIndex === -1 ? 0 : foundModeIndex];

  /** All session channels, ascending by recency (newest last). */
  const sessions = React.useMemo(() => {
    const streams = (channelsQuery.data ?? []).filter(
      (channel) =>
        channel.channelType === "stream" &&
        channel.isMember &&
        channel.archivedAt === null,
    );
    streams.sort((left, right) =>
      (left.lastMessageAt ?? "").localeCompare(right.lastMessageAt ?? ""),
    );
    return streams;
  }, [channelsQuery.data]);

  const findChannel = React.useCallback(
    (channelId: string | null) =>
      (channelsQuery.data ?? []).find((channel) => channel.id === channelId) ??
      null,
    [channelsQuery.data],
  );

  const activeChannel =
    view === "channel" ? findChannel(activeSessionId) : null;
  const previewChannel = view === "navigator" ? findChannel(navigatorId) : null;
  // A stored id whose channel vanished (or is still propagating) renders as
  // the fresh-session state; navigation starts from what is actually shown.
  const effectiveSessionId = activeChannel?.id ?? null;

  // Shares the transcript's query cache — used only for card navigation.
  const messagesQuery = useChannelMessagesQuery(activeChannel);
  const roots = React.useMemo(
    () => selectRootEvents(messagesQuery.data),
    [messagesQuery.data],
  );
  const selectedRoot = roots.find((root) => root.id === selectedRootId) ?? null;

  // Card selection and the side chat belong to one channel's transcript.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — selection resets only on channel switch
  React.useEffect(() => {
    setSelectedRootId(null);
    setThreadOpen(false);
    setActivePane("main");
  }, [effectiveSessionId]);

  // Ctrl+O opens the palette from anywhere in the shell.
  React.useEffect(() => {
    const handleWindowKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey && !event.metaKey && event.key.toLowerCase() === "o") {
        event.preventDefault();
        setPaletteOpen((current) => !current);
      }
    };
    window.addEventListener("keydown", handleWindowKeyDown);
    return () => window.removeEventListener("keydown", handleWindowKeyDown);
  }, []);

  const closePalette = React.useCallback(() => {
    setPaletteOpen(false);
    focusComposer();
  }, [focusComposer]);

  const openChannel = React.useCallback(
    (channelId: string) => {
      setActiveSessionId(channelId);
      setNavigatorId(channelId);
      setView("channel");
      focusComposer();
    },
    [focusComposer],
  );

  const goToFresh = React.useCallback(() => {
    setView("fresh");
    setActiveSessionId(null);
    setThreadOpen(false);
    setSelectedRootId(null);
    focusComposer();
  }, [focusComposer]);

  const handleCycleMode = React.useCallback(
    (direction: 1 | -1) => {
      if (modes.length === 0) return;
      const currentIndex = modes.findIndex(
        (candidate) => devComposerModeKey(candidate) === modeKey,
      );
      const baseIndex = currentIndex === -1 ? 0 : currentIndex;
      const nextIndex = (baseIndex + direction + modes.length) % modes.length;
      setModeKey(devComposerModeKey(modes[nextIndex]));
    },
    [modeKey, modes],
  );

  const navigateChannels = React.useCallback(
    (direction: 1 | -1) => {
      if (sessions.length === 0) return;
      const currentIndex = sessions.findIndex(
        (session) => session.id === navigatorId,
      );
      if (currentIndex === -1) {
        setNavigatorId(sessions[sessions.length - 1].id);
        return;
      }
      // ↑ walks toward older channels; ↓ toward newer. The navigator stays
      // open at the ends — only Enter or Escape leave it.
      const nextIndex = Math.min(
        sessions.length - 1,
        Math.max(0, currentIndex + direction),
      );
      setNavigatorId(sessions[nextIndex].id);
    },
    [navigatorId, sessions],
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
            sessions.length > 0 ? sessions[sessions.length - 1].id : null,
          );
        }
        return;
      }
      navigateChannels(direction);
    },
    [navigateChannels, navigateCards, sessions, view],
  );

  const handleEscape = React.useCallback(() => {
    if (view === "channel") {
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
      // Back out to the navigator with the current channel highlighted.
      setNavigatorId(activeSessionId);
      setActiveSessionId(null);
      setView("navigator");
      return;
    }
    if (view === "navigator") {
      goToFresh();
    }
  }, [
    activeSessionId,
    focusComposer,
    goToFresh,
    selectedRootId,
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

  const handleOpenThread = React.useCallback((rootId: string) => {
    setSelectedRootId(rootId);
    setThreadOpen(true);
    setActivePane("thread");
  }, []);

  const handleSubmit = React.useCallback(() => {
    const prompt = input.trim();
    if (!prompt) {
      if (view === "navigator" && navigatorId) {
        openChannel(navigatorId);
        return;
      }
      // Empty-input Enter opens the selected card's side chat.
      if (view === "channel" && selectedRootId) {
        handleOpenThread(selectedRootId);
      }
      return;
    }
    if (busy || !mode) return;

    setBusy(true);
    setError(null);
    setInput("");
    void (async () => {
      try {
        let channel = activeChannel;
        if (!channel) {
          channel = await createSessionChannel(prompt);
          setActiveSessionId(channel.id);
          setNavigatorId(channel.id);
          setView("channel");
        }
        await sendToSession(channel, prompt, mode);
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
  }, [
    activeChannel,
    busy,
    createSessionChannel,
    handleOpenThread,
    input,
    mode,
    navigatorId,
    openChannel,
    selectedRootId,
    sendToSession,
    view,
  ]);

  const handleThreadSend = React.useCallback(
    async (prompt: string) => {
      if (!activeChannel || !selectedRoot || !mode) {
        throw new Error("Thread is no longer available.");
      }
      await sendToSession(activeChannel, prompt, mode, selectedRoot.id);
    },
    [activeChannel, mode, selectedRoot, sendToSession],
  );

  const placeholder = activeChannel
    ? mode?.kind === "agent"
      ? `Message # ${activeChannel.name} and put ${devComposerModeLabel(mode)} to work…`
      : `Message # ${activeChannel.name}…`
    : mode?.kind === "agent"
      ? `Prompt ${devComposerModeLabel(mode)} — spawns a new channel where it works…`
      : "Start a discussion — spawns a new channel for humans…";

  const hint =
    view === "navigator"
      ? "↑↓: preview channels · enter: open · esc: back · ⌃O: palette"
      : view === "channel"
        ? threadOpen
          ? "←→: switch pane · tab: target · esc: close side chat"
          : selectedRootId
            ? "↑↓: prompts · enter: side chat · esc: back"
            : "tab: target · enter: send · ↑↓: prompts · esc: channels"
        : "tab: target · enter: send · ↑: channels · /: palette";

  const composerActive =
    !paletteOpen && !(threadOpen && activePane === "thread");

  const transcriptFor = (channel: NonNullable<typeof activeChannel>) => (
    <DevTranscript
      channel={channel}
      currentPubkey={identityQuery.data?.pubkey ?? null}
      onOpenThread={handleOpenThread}
      onSelectRoot={setSelectedRootId}
      selectedRootId={view === "channel" ? selectedRootId : null}
    />
  );

  const sideChatOpen = Boolean(
    view === "channel" && activeChannel && threadOpen && selectedRoot && mode,
  );

  const composer = mode ? (
    <DevPromptComposer
      active={composerActive}
      busy={busy}
      focusSignal={focusSignal}
      hint={hint}
      mode={mode}
      onChange={setInput}
      onCycleMode={handleCycleMode}
      onEscape={handleEscape}
      onNavigate={handleNavigate}
      onOpenPalette={() => setPaletteOpen(true)}
      onSubmit={handleSubmit}
      onSwitchPane={handleSwitchPane}
      placeholder={placeholder}
      value={input}
    />
  ) : null;

  return (
    <div
      className="relative flex min-h-0 flex-1 flex-col bg-background"
      data-testid="dev-mode-shell"
    >
      <div className="flex shrink-0 items-center justify-between border-b border-border/60 px-4 py-1.5 font-mono text-xs text-muted-foreground">
        <span>buzz · developer mode</span>
        <div className="flex items-center gap-3">
          <button
            className="cursor-pointer hover:text-foreground"
            onClick={() => setPaletteOpen(true)}
            type="button"
          >
            palette ⌃O
          </button>
          <button
            className="cursor-pointer hover:text-foreground"
            onClick={() => setDisplayStyle("standard")}
            type="button"
          >
            standard ui ⌘⇧D
          </button>
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        {view === "navigator" ? (
          <DevChannelNavigator
            channels={sessions}
            highlightedId={navigatorId}
            onHighlight={setNavigatorId}
            onOpen={openChannel}
          />
        ) : null}

        {view === "navigator" && previewChannel ? (
          <div className="pointer-events-none flex min-h-0 min-w-0 flex-1 flex-col opacity-70">
            <div className="shrink-0 border-b border-border/60 px-4 py-1 font-mono text-xs text-muted-foreground/60">
              preview — enter to open
            </div>
            {transcriptFor(previewChannel)}
          </div>
        ) : view === "channel" && activeChannel ? (
          threadOpen && selectedRoot && mode ? (
            <DevSplitPane
              activePane={activePane}
              main={
                <>
                  {transcriptFor(activeChannel)}
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
            <div className="flex min-h-0 min-w-0 flex-1 flex-col">
              {transcriptFor(activeChannel)}
            </div>
          )
        ) : (
          <div className="flex min-h-0 flex-1 items-center justify-center px-8 font-mono text-sm text-muted-foreground">
            <div className="max-w-lg space-y-2">
              <div className="text-foreground">new session</div>
              <div>
                Type a prompt and hit enter — it spawns a channel and puts the
                selected target to work. Tab cycles between chat and{" "}
                {modes.length - 1} agent{modes.length === 2 ? "" : "s"}. Press
                ⌃O for the command palette.
              </div>
            </div>
          </div>
        )}
      </div>

      {error ? (
        <div className="border-t border-destructive/40 bg-destructive/10 px-4 py-1.5 font-mono text-xs text-destructive">
          {error}
        </div>
      ) : null}

      {sideChatOpen ? null : composer}

      {paletteOpen ? (
        <DevCommandPalette
          channels={[...sessions].reverse()}
          myPubkey={identityQuery.data?.pubkey ?? null}
          onClose={closePalette}
          onNewSession={goToFresh}
          onOpenChannel={openChannel}
        />
      ) : null}
    </div>
  );
}

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
import { DevPromptComposer } from "@/features/dev-mode/ui/DevPromptComposer";
import { DevSessionList } from "@/features/dev-mode/ui/DevSessionList";
import { DevThreadPanel } from "@/features/dev-mode/ui/DevThreadPanel";
import { DevTranscript } from "@/features/dev-mode/ui/DevTranscript";
import { useChannelMessagesQuery } from "@/features/messages/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";

const SESSION_LIST_LIMIT = 20;

/**
 * Stable identity for the cycled mode. Selection is keyed rather than
 * indexed so agent list refreshes cannot silently retarget the next prompt
 * at a different agent; a vanished agent falls back to chat.
 */
function devComposerModeKey(mode: DevComposerMode): string {
  return mode.kind === "chat" ? "chat" : normalizePubkey(mode.target.pubkey);
}

export function DevModeShell() {
  const identityQuery = useIdentityQuery();
  const channelsQuery = useChannelsQuery();
  const modes = useDevComposerModes();
  const { createSessionChannel, sendToSession } = useDevSessionActions(
    identityQuery.data,
  );

  const [input, setInput] = React.useState("");
  const [modeKey, setModeKey] = React.useState("chat");
  const [activeSessionId, setActiveSessionId] = React.useState<string | null>(
    null,
  );
  const [selectedRootId, setSelectedRootId] = React.useState<string | null>(
    null,
  );
  const [threadOpen, setThreadOpen] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const foundModeIndex = modes.findIndex(
    (candidate) => devComposerModeKey(candidate) === modeKey,
  );
  const modeIndex = foundModeIndex === -1 ? 0 : foundModeIndex;
  const mode = modes[modeIndex];

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
    return streams.slice(-SESSION_LIST_LIMIT);
  }, [channelsQuery.data]);

  const activeChannel =
    (channelsQuery.data ?? []).find(
      (channel) => channel.id === activeSessionId,
    ) ?? null;
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
  }, [effectiveSessionId]);

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

  const navigateSessions = React.useCallback(
    (direction: 1 | -1) => {
      if (sessions.length === 0) return;
      const currentIndex = sessions.findIndex(
        (session) => session.id === effectiveSessionId,
      );
      if (currentIndex === -1) {
        // From the fresh-prompt state, ArrowUp enters the list at the newest
        // session; ArrowDown stays on the fresh prompt.
        if (direction === -1) {
          setActiveSessionId(sessions[sessions.length - 1].id);
        }
        return;
      }
      const nextIndex = currentIndex + direction;
      if (nextIndex >= sessions.length) {
        setActiveSessionId(null);
        return;
      }
      setActiveSessionId(sessions[Math.max(0, nextIndex)].id);
    },
    [effectiveSessionId, sessions],
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
      if (activeChannel) {
        navigateCards(direction);
      } else {
        navigateSessions(direction);
      }
    },
    [activeChannel, navigateCards, navigateSessions],
  );

  const handleEscape = React.useCallback(() => {
    if (threadOpen) {
      setThreadOpen(false);
      return;
    }
    if (selectedRootId) {
      setSelectedRootId(null);
      return;
    }
    setActiveSessionId(null);
  }, [selectedRootId, threadOpen]);

  const handleOpenThread = React.useCallback((rootId: string) => {
    setSelectedRootId(rootId);
    setThreadOpen(true);
  }, []);

  const handleSubmit = React.useCallback(() => {
    const prompt = input.trim();
    if (!prompt) {
      // Empty-input Enter opens the selected card's side chat.
      if (activeChannel && selectedRootId) {
        setThreadOpen(true);
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
    input,
    mode,
    selectedRootId,
    sendToSession,
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

  const hint = activeChannel
    ? selectedRootId
      ? "tab: switch target · ↑↓: prompts · enter: side chat · esc: back"
      : "tab: switch target · enter: send · ↑↓: prompts · esc: new session"
    : "tab: switch target · enter: send · ↑↓: sessions";

  return (
    <div
      className="flex min-h-0 flex-1 flex-col bg-background"
      data-testid="dev-mode-shell"
    >
      <div className="flex shrink-0 items-center justify-between border-b border-border/60 px-4 py-1.5 font-mono text-xs text-muted-foreground">
        <span>buzz · developer mode</span>
        <button
          className="cursor-pointer hover:text-foreground"
          onClick={() => setDisplayStyle("standard")}
          type="button"
        >
          standard ui ⌘⇧D
        </button>
      </div>

      <DevSessionList
        activeSessionId={effectiveSessionId}
        onSelect={setActiveSessionId}
        sessions={sessions}
      />

      {activeChannel ? (
        <div className="flex min-h-0 flex-1">
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <DevTranscript
              channel={activeChannel}
              currentPubkey={identityQuery.data?.pubkey ?? null}
              onOpenThread={handleOpenThread}
              onSelectRoot={setSelectedRootId}
              selectedRootId={selectedRootId}
            />
          </div>
          {threadOpen && selectedRoot && mode ? (
            <DevThreadPanel
              channel={activeChannel}
              currentPubkey={identityQuery.data?.pubkey ?? null}
              mode={mode}
              onClose={() => setThreadOpen(false)}
              onCycleMode={handleCycleMode}
              onSend={handleThreadSend}
              root={selectedRoot}
            />
          ) : null}
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 items-center justify-center px-8 font-mono text-sm text-muted-foreground">
          <div className="max-w-lg space-y-2">
            <div className="text-foreground">new session</div>
            <div>
              Type a prompt and hit enter — it spawns a channel and puts the
              selected target to work. Tab cycles between chat and{" "}
              {modes.length - 1} agent{modes.length === 2 ? "" : "s"}.
            </div>
          </div>
        </div>
      )}

      {error ? (
        <div className="border-t border-destructive/40 bg-destructive/10 px-4 py-1.5 font-mono text-xs text-destructive">
          {error}
        </div>
      ) : null}

      {mode ? (
        <DevPromptComposer
          busy={busy}
          hint={hint}
          mode={mode}
          onChange={setInput}
          onCycleMode={handleCycleMode}
          onEscape={handleEscape}
          onNavigate={handleNavigate}
          onSubmit={handleSubmit}
          placeholder={placeholder}
          value={input}
        />
      ) : null}
    </div>
  );
}

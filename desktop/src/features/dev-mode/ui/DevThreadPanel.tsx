import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useAuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { matchLeadingMention } from "@/features/dev-mode/lib/highlightContent";
import {
  applyMessageEdits,
  collectMessageEdits,
} from "@/features/dev-mode/lib/messageEdits";
import { collectReactions } from "@/features/dev-mode/lib/messageReactions";
import {
  byCreatedAscending,
  DEV_MESSAGE_KINDS,
} from "@/features/dev-mode/lib/transcriptRoots";
import {
  devComposerModeLabel,
  type DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";
import type { MentionRecord } from "@/features/dev-mode/lib/mentionRecords";
import { useChannelRefAutocomplete } from "@/features/dev-mode/lib/useChannelRefAutocomplete";
import { useComposerAutoGrow } from "@/features/dev-mode/lib/useComposerAutoGrow";
import { useMentionAutocomplete } from "@/features/dev-mode/lib/useMentionAutocomplete";
import {
  useMemberAgentResolver,
  useMemberNameResolver,
  type NameResolver,
} from "@/features/dev-mode/lib/useMemberNameResolver";
import { usePinnedScroll } from "@/features/dev-mode/lib/usePinnedScroll";
import { DevChannelSuggestions } from "@/features/dev-mode/ui/DevChannelSuggestions";
import { DevMentionSuggestions } from "@/features/dev-mode/ui/DevMentionSuggestions";
import { DevComposerAttachments } from "@/features/dev-mode/ui/DevComposerAttachments";
import { DevComposerModeLine } from "@/features/dev-mode/ui/DevComposerModeLine";
import { DevComposerResizeHandle } from "@/features/dev-mode/ui/DevComposerResizeHandle";
import { DevMessageRow } from "@/features/dev-mode/ui/DevMessageRow";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import { useMediaUpload } from "@/features/messages/lib/useMediaUpload";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

/** Header preview of the root prompt, minus any leading `@Name` direction. */
function rootPreview(root: RelayEvent, resolveName: NameResolver): string {
  const mentions = root.tags
    .filter((tag) => tag[0] === "p" && tag[1])
    .map((tag) => ({ name: resolveName(tag[1]), color: "" }));
  const directed = matchLeadingMention(root.content, mentions);
  return directed ? root.content.slice(directed.end) : root.content;
}

/**
 * Split-screen side chat for one prompt card: the root prompt, its thread,
 * and a composer that replies inside the thread so an agent picks the
 * conversation up with that context. A null root is a draft side chat
 * (⌘T): the first send posts a new message to the channel — exactly like
 * the channel composer — and the pane attaches to that new thread.
 */
export function DevThreadPanel({
  channel,
  root,
  mode,
  currentPubkey,
  active,
  onCycleMode,
  onCycleAgent,
  onSwitchPane,
  onSend,
  onClose,
}: {
  channel: Channel;
  root: RelayEvent | null;
  mode: DevComposerMode;
  currentPubkey: string | null;
  /** Whether this pane owns the keyboard (vs the main channel composer). */
  active: boolean;
  /** Tab — toggle chat ↔ last agent. */
  onCycleMode: () => void;
  /** ⌃Tab / ⌘Tab (+⇧ reverses) — cycle through the agents. */
  onCycleAgent: (direction: 1 | -1) => void;
  /** ArrowLeft / ArrowRight while the input is empty. */
  onSwitchPane: (pane: "main" | "thread") => void;
  /** Mentions are the `@Name`s still present in the sent text. */
  onSend: (
    prompt: string,
    mentions: MentionRecord[],
    media: ImetaMedia[],
  ) => Promise<void>;
  onClose: () => void;
}) {
  const repliesQuery = useThreadReplies(channel, root?.id ?? null);
  const resolveName = useMemberNameResolver(channel.id);
  const resolveColor = useAuthorColorResolver();
  const resolveIsAgent = useMemberAgentResolver(channel.id);
  const { scrollRef, contentRef, handleScroll } = usePinnedScroll(
    root?.id ?? "draft",
  );

  const [input, setInput] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const { textareaRef, dragging, resizeHandleProps } = useComposerAutoGrow(
    input,
    "buzz.devMode.threadComposerHeight",
  );
  const autocomplete = useChannelRefAutocomplete({
    value: input,
    onChange: setInput,
    textareaRef,
  });
  const mentionAutocomplete = useMentionAutocomplete({
    channelId: channel.id,
    selfPubkey: currentPubkey,
    value: input,
    onChange: setInput,
    textareaRef,
  });

  // biome-ignore lint/correctness/useExhaustiveDependencies: textareaRef is a stable ref from useComposerAutoGrow
  React.useEffect(() => {
    if (active) {
      textareaRef.current?.focus();
    }
  }, [active]);

  // Thread fetches include kind:40003 edit aux events (for the root too) —
  // resolve them so edited messages render their current text.
  const edits = React.useMemo(
    () => collectMessageEdits(repliesQuery.data),
    [repliesQuery.data],
  );
  const replies = React.useMemo(
    () =>
      applyMessageEdits(repliesQuery.data)
        .filter((event) => DEV_MESSAGE_KINDS.has(event.kind))
        .sort(byCreatedAscending),
    [repliesQuery.data],
  );
  const reactions = React.useMemo(
    () => collectReactions(repliesQuery.data),
    [repliesQuery.data],
  );
  const agentColor =
    mode.kind === "agent" ? resolveColor(mode.target.pubkey) : null;

  // The open side chat shows the whole thread, so advance the shared read
  // frontier to the newest loaded reply — this is what clears the thread's
  // contextual unread dots (card, tab, navigator) as replies stream in.
  const { getThreadReadAt, markThreadRead } = useAppShell();
  const rootId = root?.id ?? null;
  const latestVisibleAt = React.useMemo(() => {
    let latest = root?.created_at ?? 0;
    for (const reply of replies) {
      if (reply.created_at > latest) latest = reply.created_at;
    }
    return latest > 0 ? latest : null;
  }, [replies, root]);
  const channelId = channel.id;
  React.useEffect(() => {
    if (rootId === null || latestVisibleAt === null) return;
    const readAt = getThreadReadAt(rootId, channelId);
    if (readAt === null || readAt < latestVisibleAt) {
      markThreadRead(rootId, latestVisibleAt);
    }
  }, [channelId, getThreadReadAt, latestVisibleAt, markThreadRead, rootId]);

  // Pasted images/videos upload immediately and attach to the next send.
  const {
    handlePaste,
    isUploading,
    pendingImeta,
    removeAttachment,
    setPendingImeta,
    uploadingPreviews,
    uploadState,
  } = useMediaUpload();

  const handleSubmit = () => {
    const prompt = input.trim();
    if ((!prompt && pendingImeta.length === 0) || busy || isUploading) return;
    const mentions = mentionAutocomplete.extract(prompt);
    const media = pendingImeta;
    setBusy(true);
    setError(null);
    setInput("");
    setPendingImeta([]);
    void (async () => {
      try {
        await onSend(prompt, mentions, media);
      } catch (submitError) {
        setError(
          submitError instanceof Error
            ? submitError.message
            : "Failed to send reply.",
        );
        setInput((current) => (current === "" ? prompt : current));
        setPendingImeta((current) => (current.length === 0 ? media : current));
      } finally {
        setBusy(false);
      }
    })();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Open `#channel` / `@user` suggestions own Tab/Enter/arrows/Escape.
    if (autocomplete.handleKeyDown(event)) {
      return;
    }
    if (mentionAutocomplete.handleKeyDown(event)) {
      return;
    }

    if (event.key === "Tab") {
      event.preventDefault();
      if (event.metaKey || event.ctrlKey) {
        onCycleAgent(event.shiftKey ? -1 : 1);
      } else {
        onCycleMode();
      }
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      handleSubmit();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if ((event.key === "ArrowLeft" || event.key === "ArrowRight") && !input) {
      event.preventDefault();
      onSwitchPane(event.key === "ArrowLeft" ? "main" : "thread");
    }
  };

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col font-mono"
      data-testid="dev-mode-thread-panel"
    >
      <div className="flex shrink-0 items-center justify-between border-b border-border/60 px-3 py-1.5 text-xs text-muted-foreground">
        <span className="min-w-0 truncate">
          side chat · {root ? rootPreview(root, resolveName) : "new thread"}
        </span>
        <button
          className="ml-2 shrink-0 cursor-pointer hover:text-foreground"
          onClick={onClose}
          type="button"
        >
          esc ✕
        </button>
      </div>

      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-3 py-2"
        data-allow-text-selection
        onScroll={handleScroll}
      >
        <div ref={contentRef}>
          {root ? (
            <>
              <DevMessageRow
                event={root}
                currentPubkey={currentPubkey}
                edited={edits.has(root.id)}
                reactions={reactions.get(root.id)}
                resolveColor={resolveColor}
                resolveIsAgent={resolveIsAgent}
                resolveName={resolveName}
              />
              <div className="my-1 border-t border-border/40" />
            </>
          ) : (
            <div className="py-0.5 text-sm text-muted-foreground/60">
              new thread — your message posts to # {channel.name} and starts a
              thread here
            </div>
          )}
          {replies.map((reply) => (
            <DevMessageRow
              key={reply.localKey ?? reply.id}
              event={reply}
              currentPubkey={currentPubkey}
              edited={edits.has(reply.id)}
              reactions={reactions.get(reply.id)}
              resolveColor={resolveColor}
              resolveIsAgent={resolveIsAgent}
              resolveName={resolveName}
            />
          ))}
          {repliesQuery.isLoading ? (
            <div className="py-0.5 text-sm text-muted-foreground/60">
              loading replies…
            </div>
          ) : null}
          {repliesQuery.isError ? (
            <button
              className="cursor-pointer py-0.5 text-sm text-destructive hover:underline"
              onClick={() => void repliesQuery.refetch()}
              type="button"
            >
              failed to load replies — retry
            </button>
          ) : null}
        </div>
      </div>

      {error ? (
        <div className="border-t border-destructive/40 bg-destructive/10 px-3 py-1 text-xs text-destructive">
          {error}
        </div>
      ) : null}

      <div>
        <DevComposerResizeHandle
          dragging={dragging}
          testId="dev-mode-thread-composer-resize"
          {...resizeHandleProps}
        />
        {/* Mirror the composer row's prefix so the mode line lines up exactly
            with the textarea's left edge. */}
        <div className="flex items-start gap-2 px-3 pt-1">
          <span aria-hidden className="invisible select-none text-sm">
            ⏵
          </span>
          <DevComposerModeLine
            agentColor={agentColor}
            busy={busy}
            mode={mode}
          />
        </div>
        <DevComposerAttachments
          errorMessage={
            uploadState.status === "error"
              ? (uploadState.message ?? "upload failed")
              : null
          }
          onRemove={removeAttachment}
          pendingImeta={pendingImeta}
          uploadingPreviews={uploadingPreviews}
        />
        <div className="relative flex items-start gap-2 px-3 pt-1">
          {active && autocomplete.open ? (
            <DevChannelSuggestions
              onAccept={autocomplete.accept}
              selectedIndex={autocomplete.selectedIndex}
              suggestions={autocomplete.suggestions}
            />
          ) : active && mentionAutocomplete.open ? (
            <DevMentionSuggestions
              onAccept={mentionAutocomplete.accept}
              selectedIndex={mentionAutocomplete.selectedIndex}
              suggestions={mentionAutocomplete.suggestions}
            />
          ) : null}
          <span
            aria-hidden
            className={cn(
              "select-none pt-[3px] text-sm",
              !agentColor && "text-muted-foreground",
            )}
            style={agentColor ? { color: agentColor } : undefined}
          >
            ⏵
          </span>
          <textarea
            ref={textareaRef}
            className="min-h-6 w-full resize-none bg-transparent text-sm leading-6 outline-none placeholder:text-muted-foreground/60"
            data-testid="dev-mode-thread-composer"
            onChange={(event) => {
              setInput(event.target.value);
              autocomplete.syncCursor(event.target);
              mentionAutocomplete.syncCursor(event.target);
            }}
            onFocus={() => onSwitchPane("thread")}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            onSelect={(event) => {
              autocomplete.syncCursor(event.currentTarget);
              mentionAutocomplete.syncCursor(event.currentTarget);
            }}
            placeholder={
              root
                ? mode.kind === "agent"
                  ? `Ask ${devComposerModeLabel(mode)} about this thread…`
                  : "Reply in this thread…"
                : mode.kind === "agent"
                  ? `Prompt ${devComposerModeLabel(mode)} — starts a new thread in # ${channel.name}…`
                  : `Start a new thread in # ${channel.name}…`
            }
            rows={1}
            spellCheck={false}
            value={input}
          />
        </div>
        <div className="mt-1 flex items-center justify-end pr-3 pb-2 pl-9 text-xs text-muted-foreground">
          <span className="select-none">
            {root ? "enter: reply" : "enter: start thread"} · ←: channel · esc:
            close
          </span>
        </div>
      </div>
    </div>
  );
}

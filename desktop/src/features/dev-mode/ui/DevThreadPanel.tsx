import * as React from "react";

import { useAuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { matchLeadingMention } from "@/features/dev-mode/lib/highlightContent";
import { collectReactions } from "@/features/dev-mode/lib/messageReactions";
import {
  byCreatedAscending,
  DEV_MESSAGE_KINDS,
} from "@/features/dev-mode/lib/transcriptRoots";
import {
  devComposerModeLabel,
  type DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";
import { useChannelRefAutocomplete } from "@/features/dev-mode/lib/useChannelRefAutocomplete";
import { useComposerAutoGrow } from "@/features/dev-mode/lib/useComposerAutoGrow";
import {
  useMemberNameResolver,
  type NameResolver,
} from "@/features/dev-mode/lib/useMemberNameResolver";
import { usePinnedScroll } from "@/features/dev-mode/lib/usePinnedScroll";
import { DevChannelSuggestions } from "@/features/dev-mode/ui/DevChannelSuggestions";
import { DevComposerModeLine } from "@/features/dev-mode/ui/DevComposerModeLine";
import { DevComposerResizeHandle } from "@/features/dev-mode/ui/DevComposerResizeHandle";
import { DevMessageRow } from "@/features/dev-mode/ui/DevMessageRow";
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
  onCycleMode: (direction: 1 | -1) => void;
  /** ArrowLeft / ArrowRight while the input is empty. */
  onSwitchPane: (pane: "main" | "thread") => void;
  onSend: (prompt: string) => Promise<void>;
  onClose: () => void;
}) {
  const repliesQuery = useThreadReplies(channel, root?.id ?? null);
  const resolveName = useMemberNameResolver(channel.id);
  const resolveColor = useAuthorColorResolver();
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

  // biome-ignore lint/correctness/useExhaustiveDependencies: textareaRef is a stable ref from useComposerAutoGrow
  React.useEffect(() => {
    if (active) {
      textareaRef.current?.focus();
    }
  }, [active]);

  const replies = React.useMemo(
    () =>
      (repliesQuery.data ?? [])
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

  const handleSubmit = () => {
    const prompt = input.trim();
    if (!prompt || busy) return;
    setBusy(true);
    setError(null);
    setInput("");
    void (async () => {
      try {
        await onSend(prompt);
      } catch (submitError) {
        setError(
          submitError instanceof Error
            ? submitError.message
            : "Failed to send reply.",
        );
        setInput((current) => (current === "" ? prompt : current));
      } finally {
        setBusy(false);
      }
    })();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Open `#channel` suggestions own Tab/Enter/arrows/Escape.
    if (autocomplete.handleKeyDown(event)) {
      return;
    }

    if (event.key === "Tab") {
      event.preventDefault();
      onCycleMode(event.shiftKey ? -1 : 1);
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
        className="min-h-0 flex-1 overflow-y-auto px-3 py-2"
        data-allow-text-selection
        onScroll={handleScroll}
      >
        <div ref={contentRef}>
          {root ? (
            <>
              <DevMessageRow
                event={root}
                isSelf={root.pubkey === currentPubkey}
                reactions={reactions.get(root.id)}
                resolveColor={resolveColor}
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
              isSelf={reply.pubkey === currentPubkey}
              reactions={reactions.get(reply.id)}
              resolveColor={resolveColor}
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
        <DevComposerModeLine
          agentColor={agentColor}
          busy={busy}
          className="pt-1 pl-9"
          mode={mode}
        />
        <div className="relative flex items-start gap-2 px-3 pt-1">
          {active && autocomplete.open ? (
            <DevChannelSuggestions
              onAccept={autocomplete.accept}
              selectedIndex={autocomplete.selectedIndex}
              suggestions={autocomplete.suggestions}
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
            }}
            onFocus={() => onSwitchPane("thread")}
            onKeyDown={handleKeyDown}
            onSelect={(event) => autocomplete.syncCursor(event.currentTarget)}
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

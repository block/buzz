import * as React from "react";

import { useAuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { collectReactions } from "@/features/dev-mode/lib/messageReactions";
import {
  byCreatedAscending,
  DEV_MESSAGE_KINDS,
} from "@/features/dev-mode/lib/transcriptRoots";
import {
  devComposerModeLabel,
  type DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";
import { useMemberNameResolver } from "@/features/dev-mode/lib/useMemberNameResolver";
import { usePinnedScroll } from "@/features/dev-mode/lib/usePinnedScroll";
import { DevMessageRow } from "@/features/dev-mode/ui/DevMessageRow";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

/**
 * Split-screen side chat for one prompt card: the root prompt, its thread,
 * and a composer that replies inside the thread so an agent picks the
 * conversation up with that context.
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
  root: RelayEvent;
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
  const repliesQuery = useThreadReplies(channel, root.id);
  const resolveName = useMemberNameResolver(channel.id);
  const resolveColor = useAuthorColorResolver();
  const { scrollRef, contentRef, handleScroll } = usePinnedScroll(root.id);

  const [input, setInput] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);

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

  const rowCount = Math.min(input.split("\n").length, 6);

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col font-mono"
      data-testid="dev-mode-thread-panel"
    >
      <div className="flex shrink-0 items-center justify-between border-b border-border/60 px-3 py-1.5 text-xs text-muted-foreground">
        <span className="min-w-0 truncate">side chat · {root.content}</span>
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
        onScroll={handleScroll}
      >
        <div ref={contentRef}>
          <DevMessageRow
            event={root}
            isSelf={root.pubkey === currentPubkey}
            reactions={reactions.get(root.id)}
            resolveColor={resolveColor}
            resolveName={resolveName}
          />
          <div className="my-1 border-t border-border/40" />
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

      <div className="border-t border-border/60 px-3 py-2">
        <div className="flex items-start gap-2">
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
            onChange={(event) => setInput(event.target.value)}
            onFocus={() => onSwitchPane("thread")}
            onKeyDown={handleKeyDown}
            placeholder={
              mode.kind === "agent"
                ? `Ask ${devComposerModeLabel(mode)} about this thread…`
                : "Reply in this thread…"
            }
            rows={rowCount}
            spellCheck={false}
            value={input}
          />
        </div>
        <div className="mt-1 flex items-center justify-between pl-6 text-xs text-muted-foreground">
          <span
            className={cn(
              "rounded-none border px-1.5 py-0.5 font-medium",
              !agentColor && "border-border text-muted-foreground",
            )}
            style={
              agentColor
                ? { color: agentColor, borderColor: `${agentColor}80` }
                : undefined
            }
          >
            {busy ? "working…" : devComposerModeLabel(mode)}
          </span>
          <span className="select-none">
            enter: reply · ←: channel · esc: close
          </span>
        </div>
      </div>
    </div>
  );
}

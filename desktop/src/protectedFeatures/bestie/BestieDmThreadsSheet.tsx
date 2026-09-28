import { ChevronDown, ChevronRight, MessagesSquare, Plus, SquareArrowOutUpRight, Trash2 } from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import { getThreadReference, isThreadReply } from "@/features/messages/lib/threading";
import { getEventById, getHomeFeed } from "@/shared/api/tauri";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { ensureBestieAgentThreadAccess } from "./bestieThreadAccess";
import {
  bestieParticipatingThreadInputs,
  parseBestieThreadAddInput,
} from "./bestieThreadDiscover";
import { BESTIE_THREAD_SUMMARIZE_EVENT } from "./bestieThreadProtocol";
import { bestieThreadNeedsSummarize } from "./bestieThreadSummarizeEligibility";
import { BESTIE_THREAD_SUMMARIZE_LIVE_LABEL } from "./bestieThreadSummarizeLive";
import { sortedBestieThreads } from "./bestieThreadStorage";
import {
  beginBestieThreadSummarizeForScope,
  removeBestieTrackedThreadForScope,
  syncBestieParticipatingThreadsForScope,
  upsertBestieTrackedThreadForScope,
  useBestieThreads,
} from "./bestieThreadStore";
import type {
  BestieThreadScope,
  BestieTrackedThread,
} from "./bestieThreadTypes";
import { findBestieDmChannel } from "./filterBestieDmChannels";

function ThreadRow({
  pending,
  scope,
  summarizeLive,
  thread,
}: {
  pending: boolean;
  scope: BestieThreadScope;
  /** ACP summarize turn live for this row — show 🤔… (not coffee/other). */
  summarizeLive: boolean;
  thread: BestieTrackedThread;
}) {
  const { goChannel } = useAppNavigation();
  // Rows start collapsed; click the thread header to expand and read summary.
  const [expanded, setExpanded] = React.useState(false);
  const needsSummarize = bestieThreadNeedsSummarize(thread);
  const title =
    thread.channelName?.trim() ||
    `Thread ${thread.rootEventId.slice(0, 8)}…`;
  const sourceLabel =
    thread.source === "agent"
      ? "In thread"
      : thread.source === "add"
        ? "Added"
        : "Ask Assistant";
  const Chevron = expanded ? ChevronDown : ChevronRight;

  return (
    <div
      className="rounded-md border border-border/60 bg-muted/25 px-2 py-1.5"
      data-testid={`bestie-thread-item-${thread.id}`}
    >
      <div className="flex items-start gap-2">
        <button
          aria-expanded={expanded}
          className="min-w-0 flex-1 text-left"
          data-testid={`bestie-thread-toggle-${thread.id}`}
          onClick={() => setExpanded((value) => !value)}
          type="button"
        >
          <p className="flex items-center gap-1 text-sm font-medium leading-snug">
            <Chevron className="size-3.5 shrink-0 text-muted-foreground" />
            <span className="truncate">{title}</span>
          </p>
          <p className="mt-0.5 line-clamp-2 pl-5 text-2xs text-muted-foreground">
            {thread.preview || "No preview"}
          </p>
          <p className="mt-0.5 pl-5 text-2xs text-muted-foreground">
            {sourceLabel} · Added{" "}
            {new Date(thread.addedAt * 1000).toLocaleString()}
            {thread.lastSummaryAt
              ? ` · summarized ${new Date(thread.lastSummaryAt * 1000).toLocaleString()}`
              : ""}
          </p>
        </button>
        {needsSummarize || summarizeLive ? (
          <Button
            className="h-7 shrink-0 px-2 text-xs"
            data-testid={`bestie-thread-summarize-${thread.id}`}
            disabled={pending}
            onClick={() => {
              const begun = beginBestieThreadSummarizeForScope(
                scope,
                thread.id,
              );
              if (!begun) return;
              window.dispatchEvent(
                new CustomEvent(BESTIE_THREAD_SUMMARIZE_EVENT, {
                  detail: { threadId: thread.id },
                }),
              );
            }}
            size="sm"
            type="button"
            variant="secondary"
          >
            {summarizeLive
              ? BESTIE_THREAD_SUMMARIZE_LIVE_LABEL
              : pending
                ? "…"
                : "Summarize"}
          </Button>
        ) : null}
        <Button
          aria-label="Open thread"
          className="size-6 shrink-0"
          data-testid={`bestie-thread-open-${thread.id}`}
          onClick={() =>
            void goChannel(thread.channelId, { thread: thread.rootEventId })
          }
          size="icon-xs"
          type="button"
          variant="ghost"
        >
          <SquareArrowOutUpRight className="size-3.5" />
        </Button>
        <Button
          aria-label="Stop tracking thread"
          className="size-6 shrink-0"
          data-testid={`bestie-thread-remove-${thread.id}`}
          onClick={() => removeBestieTrackedThreadForScope(scope, thread.id)}
          size="icon-xs"
          type="button"
          variant="ghost"
        >
          <Trash2 className="size-3.5" />
        </Button>
      </div>
      {expanded ? (
        <div className="mt-1.5 border-t border-border/40 pt-1.5 pl-5">
          {thread.lastSummary ? (
            <p
              className="whitespace-pre-wrap text-sm leading-snug text-muted-foreground"
              data-testid={`bestie-thread-summary-${thread.id}`}
            >
              {thread.lastSummary}
            </p>
          ) : (
            <p className="text-xs text-muted-foreground">
              No summary yet
              {needsSummarize ? " — use Summarize when ready." : "."}
            </p>
          )}
        </div>
      ) : null}
    </div>
  );
}

function AddThreadRow({
  busy,
  error,
  onSubmit,
}: {
  busy: boolean;
  error: string | null;
  onSubmit: (raw: string) => void;
}) {
  const [value, setValue] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);
  React.useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const submit = () => {
    if (!value.trim() || busy) return;
    onSubmit(value);
  };

  return (
    <div className="flex flex-col gap-1.5" data-testid="bestie-add-thread">
      <Input
        aria-label="Thread link or channel and event id"
        className="h-8 text-sm"
        disabled={busy}
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            submit();
          }
        }}
        placeholder="buzz://message?… or channelId eventId"
        ref={inputRef}
        value={value}
      />
      <div className="flex items-center gap-1.5">
        <p className="min-w-0 flex-1 text-2xs text-muted-foreground">
          Adds a thread Assistant is not in yet, invites to private channels if
          needed, then summarizes.
        </p>
        <Button
          aria-label="Add thread and summarize"
          className="size-8 shrink-0"
          data-testid="bestie-add-thread-submit"
          disabled={!value.trim() || busy}
          onClick={submit}
          size="icon"
          type="button"
          variant="secondary"
        >
          <Plus className="size-4" />
        </Button>
      </div>
      {error ? (
        <p
          className="text-2xs text-destructive"
          data-testid="bestie-add-thread-error"
        >
          {error}
        </p>
      ) : null}
    </div>
  );
}

/** Slide sheet for Threads — participation list + add/summarize. */
export function BestieDmThreadsSheet({
  adding,
  onRequestAdd,
  scope,
  summarizeDisabled = false,
  summarizeLive = false,
  summarizeLiveThreadId = null,
}: {
  adding: boolean;
  onRequestAdd?: () => void;
  scope: BestieThreadScope;
  /** Disable Summarize while a summarize turn is live / in start grace. */
  summarizeDisabled?: boolean;
  /** True while summarize ACP turn is live — sheet header 🤔…. */
  summarizeLive?: boolean;
  summarizeLiveThreadId?: string | null;
}) {
  const state = useBestieThreads(scope);
  const threads = sortedBestieThreads(state);
  const pendingId = state.pendingSummarize?.threadId ?? null;
  const channelsQuery = useChannelsQuery();
  const [addBusy, setAddBusy] = React.useState(false);
  const [addError, setAddError] = React.useState<string | null>(null);

  // Sync threads the Assistant agent is already part of (authored in).
  React.useEffect(() => {
    let cancelled = false;
    const run = async () => {
      try {
        const feed = await getHomeFeed({
          limit: 80,
          types: "agent_activity",
        });
        if (cancelled) return;
        const dm = findBestieDmChannel(
          channelsQuery.data ?? [],
          scope.ownerPubkey,
          scope.agentPubkey,
        );
        const inputs = bestieParticipatingThreadInputs({
          agentPubkey: scope.agentPubkey,
          excludeChannelIds: dm ? [dm.id] : [],
          items: feed.feed.agentActivity,
        });
        if (inputs.length > 0) {
          syncBestieParticipatingThreadsForScope(scope, inputs);
        }
      } catch {
        // Best-effort discovery; Ask Assistant + manual + still work.
      }
    };
    void run();
    return () => {
      cancelled = true;
    };
  }, [channelsQuery.data, scope]);

  const handleAdd = React.useCallback(
    async (raw: string) => {
      setAddError(null);
      const parsed = parseBestieThreadAddInput(raw);
      if (!parsed) {
        setAddError(
          "Paste a buzz://message link, or channelId and event id.",
        );
        return;
      }
      setAddBusy(true);
      try {
        const access = await ensureBestieAgentThreadAccess({
          agentPubkey: scope.agentPubkey,
          channelId: parsed.channelId,
          ownerPubkey: scope.ownerPubkey,
        });
        if (!access.ok) {
          setAddError(access.error);
          return;
        }

        let preview = "";
        let rootEventId = parsed.rootEventId;
        try {
          const event = await getEventById(parsed.messageId);
          preview = (event.content ?? "").trim().slice(0, 280);
          const tags = Array.isArray(event.tags) ? event.tags : [];
          if (isThreadReply(tags)) {
            rootEventId =
              getThreadReference(tags).rootId ?? parsed.rootEventId;
          } else if (
            !parsed.rootEventId ||
            parsed.rootEventId === parsed.messageId
          ) {
            rootEventId = event.id;
          }
        } catch {
          // Preview optional — still enroll with ids from the paste.
        }

        const channel = (channelsQuery.data ?? []).find(
          (entry) => entry.id === parsed.channelId,
        );
        const next = upsertBestieTrackedThreadForScope(scope, {
          channelId: parsed.channelId,
          channelName: channel?.name ?? null,
          preview,
          rootEventId,
          source: "add",
        });
        const thread = next.threads.find(
          (entry) =>
            entry.channelId === parsed.channelId &&
            entry.rootEventId === rootEventId,
        );
        if (!thread) return;
        const begun = beginBestieThreadSummarizeForScope(scope, thread.id);
        if (!begun) {
          setAddError("A summarize is already in progress.");
          return;
        }
        window.dispatchEvent(
          new CustomEvent(BESTIE_THREAD_SUMMARIZE_EVENT, {
            detail: { threadId: thread.id },
          }),
        );
      } finally {
        setAddBusy(false);
      }
    },
    [channelsQuery.data, scope],
  );

  return (
    <div
      className="flex flex-col gap-3 py-1"
      data-testid="bestie-dm-threads-sheet"
    >
      {adding ? (
        <AddThreadRow
          busy={addBusy}
          error={addError}
          onSubmit={(raw) => void handleAdd(raw)}
        />
      ) : (
        <p className="px-0.5 text-xs text-muted-foreground">
          Threads Assistant is in (replies) plus Ask Assistant enrollments.
          Use + to add one they are not in yet and summarize.{" "}
          {onRequestAdd ? (
            <button
              className="underline-offset-2 hover:underline"
              onClick={onRequestAdd}
              type="button"
            >
              Add one
            </button>
          ) : null}
        </p>
      )}
      <div className="space-y-1.5">
        {threads.length === 0 ? (
          <p className="px-0.5 text-xs text-muted-foreground">
            No tracked threads yet. Ask Assistant on a message, wait for
            Assistant to reply in a channel thread, or use +.
          </p>
        ) : (
          threads.map((thread) => (
            <ThreadRow
              key={thread.id}
              pending={
                summarizeDisabled ||
                pendingId === thread.id ||
                summarizeLiveThreadId === thread.id
              }
              scope={scope}
              summarizeLive={
                summarizeLive && summarizeLiveThreadId === thread.id
              }
              thread={thread}
            />
          ))
        )}
      </div>
      {summarizeLive ? (
        <p
          className="flex items-center gap-1.5 text-xs text-muted-foreground"
          data-testid="bestie-thread-summarizing"
        >
          <MessagesSquare className="size-3.5" />
          Summarizing…
        </p>
      ) : null}
    </div>
  );
}

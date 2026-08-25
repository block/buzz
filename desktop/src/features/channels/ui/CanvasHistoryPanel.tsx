import { diffLines } from "diff";
import { RotateCcw } from "lucide-react";
import * as React from "react";

import {
  useCanvasHistoryQuery,
  useSetCanvasMutation,
} from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { CanvasRevision } from "@/shared/api/types";
import { formatItemTimestamp } from "@/shared/lib/datetime";
import { truncatePubkey } from "@/shared/lib/pubkey";
import {
  isRelayUnreachableError,
  RELAY_UNREACHABLE_SHORT,
} from "@/shared/lib/relayError";
import {
  CANVAS_CONFLICT_MESSAGE,
  CANVAS_EXPECTED_REVISION_NONE,
  isCanvasConflictError,
} from "@/features/channels/canvasConflict";
import { Button } from "@/shared/ui/button";

type CanvasHistoryPanelProps = {
  channelId: string;
  currentContent: string;
  currentRevision: string | null;
  canRestore: boolean;
};

/**
 * Revision history for a channel canvas. Every kind:40100 write is a regular
 * signed event the relay retains, so the list is the complete edit stream —
 * newest first, the head marked "Current". Selecting an older revision reveals
 * a line diff against the current content and (when the viewer can edit) a
 * Restore action.
 *
 * Restore never mutates history: it publishes a new head carrying the selected
 * revision's content, guarded by `expected-revision` = the current head so a
 * concurrent edit surfaces the same conflict state as a normal save.
 */
export function CanvasHistoryPanel({
  channelId,
  currentContent,
  currentRevision,
  canRestore,
}: CanvasHistoryPanelProps) {
  const historyQuery = useCanvasHistoryQuery(channelId, true);
  const restoreMutation = useSetCanvasMutation(channelId);
  const [selectedId, setSelectedId] = React.useState<string | null>(null);

  const revisions = React.useMemo(
    () => historyQuery.data?.pages.flatMap((page) => page.revisions) ?? [],
    [historyQuery.data],
  );
  const authorPubkeys = React.useMemo(
    () => revisions.map((revision) => revision.author),
    [revisions],
  );
  const profilesQuery = useUsersBatchQuery(authorPubkeys, {
    enabled: authorPubkeys.length > 0,
  });

  function authorLabel(pubkey: string): string {
    const summary = profilesQuery.data?.profiles[pubkey.toLowerCase()];
    return summary?.displayName?.trim() || truncatePubkey(pubkey);
  }

  async function handleRestore(revision: CanvasRevision) {
    // Restore is a conflict-checked publish against the live head: if the
    // canvas moved since this panel loaded, the relay rejects and we surface
    // the same reload state as a normal save.
    await restoreMutation.mutateAsync({
      content: revision.content,
      expectedRevision: currentRevision ?? CANVAS_EXPECTED_REVISION_NONE,
    });
    setSelectedId(null);
  }

  if (historyQuery.isLoading) {
    return <p className="text-sm text-muted-foreground">Loading history...</p>;
  }

  if (historyQuery.error instanceof Error) {
    return (
      <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
        {isRelayUnreachableError(historyQuery.error)
          ? RELAY_UNREACHABLE_SHORT
          : historyQuery.error.message}
      </p>
    );
  }

  if (revisions.length === 0) {
    return <p className="text-sm text-muted-foreground">No revisions yet.</p>;
  }

  return (
    <div className="space-y-2" data-testid="channel-canvas-history">
      <ul className="space-y-2">
        {revisions.map((revision) => {
          const isCurrent = revision.eventId === currentRevision;
          const isSelected = revision.eventId === selectedId;
          return (
            <li
              className="rounded-xl border border-border/70 bg-muted/10"
              data-testid="channel-canvas-history-item"
              key={revision.eventId}
            >
              <button
                aria-expanded={isSelected}
                className="flex w-full items-baseline justify-between gap-2 px-3 py-2 text-left"
                onClick={() => {
                  // Clear any prior restore error so it can't render under a
                  // different row once the selection moves — the mutation state
                  // is shared across every row.
                  restoreMutation.reset();
                  setSelectedId(isSelected ? null : revision.eventId);
                }}
                type="button"
              >
                <span className="truncate text-sm font-medium">
                  {authorLabel(revision.author)}
                  {isCurrent ? (
                    <span className="ml-2 text-xs font-normal text-muted-foreground">
                      Current
                    </span>
                  ) : null}
                </span>
                <span className="shrink-0 text-xs text-muted-foreground">
                  {formatItemTimestamp(revision.createdAt, { withTime: true })}
                </span>
              </button>
              {isSelected ? (
                <div className="space-y-2 border-t border-border/70 px-3 py-2">
                  <CanvasRevisionDiff
                    current={currentContent}
                    revision={revision.content}
                  />
                  {canRestore && !isCurrent ? (
                    <Button
                      data-testid="channel-canvas-restore"
                      disabled={restoreMutation.isPending}
                      onClick={() => {
                        void handleRestore(revision).catch(() => {
                          // Surfaced below via restoreMutation.error.
                        });
                      }}
                      size="sm"
                      type="button"
                      variant="outline"
                    >
                      <RotateCcw className="h-4 w-4" />
                      {restoreMutation.isPending
                        ? "Restoring..."
                        : "Restore this revision"}
                    </Button>
                  ) : null}
                  {restoreMutation.error instanceof Error ? (
                    <p className="text-sm text-destructive">
                      {isCanvasConflictError(restoreMutation.error)
                        ? CANVAS_CONFLICT_MESSAGE
                        : restoreMutation.error.message}
                    </p>
                  ) : null}
                </div>
              ) : null}
            </li>
          );
        })}
      </ul>
      {historyQuery.hasNextPage ? (
        <Button
          data-testid="channel-canvas-history-load-older"
          disabled={historyQuery.isFetchingNextPage}
          onClick={() => void historyQuery.fetchNextPage()}
          size="sm"
          type="button"
          variant="ghost"
        >
          {historyQuery.isFetchingNextPage ? "Loading..." : "Load older"}
        </Button>
      ) : null}
    </div>
  );
}

/**
 * Line-level diff of a past revision against the current canvas content.
 * Additions are the revision's lines not in current; removals are current
 * lines the revision drops. Unchanged runs render muted for context.
 */
function CanvasRevisionDiff({
  current,
  revision,
}: {
  current: string;
  revision: string;
}) {
  const parts = React.useMemo(
    () => diffLines(current, revision),
    [current, revision],
  );
  if (parts.length === 1 && !parts[0].added && !parts[0].removed) {
    return (
      <p className="text-xs text-muted-foreground">
        Identical to the current canvas.
      </p>
    );
  }
  // Each part covers a distinct, non-overlapping slice of the concatenated
  // diff, so its cumulative character offset is a stable, unique key.
  let offset = 0;
  return (
    <pre
      className="max-h-64 overflow-auto rounded-lg bg-background/60 p-2 font-mono text-xs leading-relaxed"
      data-testid="channel-canvas-diff"
    >
      {parts.map((part) => {
        const prefix = part.added ? "+" : part.removed ? "-" : " ";
        const tone = part.added
          ? "text-emerald-600 dark:text-emerald-400"
          : part.removed
            ? "text-destructive"
            : "text-muted-foreground";
        const key = `${prefix}${offset}`;
        offset += part.value.length;
        return (
          <span className={tone} key={key}>
            {part.value
              .replace(/\n$/, "")
              .split("\n")
              .map((line) => `${prefix} ${line}`)
              .join("\n")}
            {"\n"}
          </span>
        );
      })}
    </pre>
  );
}

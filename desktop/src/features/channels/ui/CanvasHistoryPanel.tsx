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
  CANVAS_EXPECTED_REVISION_NONE,
  canvasConflictMessage,
} from "@/features/channels/canvasConflict";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
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
  // Restore rewrites the shared channel canvas for everyone, so it is gated
  // behind an explicit confirmation identifying the target revision. Holds the
  // revision awaiting confirmation and the head revision captured at open time;
  // `null` means no dialog is open. Nothing mutates until the user confirms.
  //
  // `frozenExpectedRevision` is snapshotted at dialog-open rather than read
  // from the current render at confirm-time. Without this, a background
  // refetch that installs a new head between "open" and "confirm" would
  // silently submit the newer head as the CAS guard, bypassing the conflict
  // check for the user's original intent.
  const [confirmRevision, setConfirmRevision] = React.useState<{
    revision: CanvasRevision;
    frozenExpectedRevision: string | null;
  } | null>(null);
  // Non-destructive notice shown after a restore the relay accepted but could
  // not verify (the post-write supersession read failed). The restore is
  // durable; the note tells the user to check the current canvas if a
  // concurrent edit later appears. Cleared whenever the selection changes.
  const [unverifiedRestoreNotice, setUnverifiedRestoreNotice] =
    React.useState(false);
  // After a restore the selection collapses and the focused Restore button
  // unmounts. Move focus to the most informative surviving destination: the
  // unverified notice when it renders, otherwise the toggle of the row that was
  // just restored. `pendingRestoreFocus` arms the move; the effect runs it once
  // the collapsed tree paints.
  const noticeRef = React.useRef<HTMLParagraphElement | null>(null);
  const restoredRowRef = React.useRef<HTMLButtonElement | null>(null);
  const [restoredId, setRestoredId] = React.useState<string | null>(null);
  const [pendingRestoreFocus, setPendingRestoreFocus] = React.useState(false);
  React.useEffect(() => {
    if (pendingRestoreFocus && selectedId === null) {
      (noticeRef.current ?? restoredRowRef.current)?.focus();
      setPendingRestoreFocus(false);
    }
  }, [pendingRestoreFocus, selectedId]);

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

  async function handleRestore(
    revision: CanvasRevision,
    frozenExpectedRevision: string | null,
  ) {
    // Restore is a conflict-checked publish against the head that was live when
    // the user opened the confirmation dialog. Using the frozen value rather
    // than the current render's `currentRevision` prevents a background refetch
    // that lands a new head between "open" and "confirm" from silently advancing
    // the CAS guard past the user's decision point.
    const result = await restoreMutation.mutateAsync({
      content: revision.content,
      expectedRevision: frozenExpectedRevision ?? CANVAS_EXPECTED_REVISION_NONE,
    });
    // The restore was accepted. `verified: false` means the post-write
    // supersession read failed, not that the restore failed — collapse the
    // selection and surface the same non-destructive note as an unverified
    // save. A detected supersession rejects the promise (handled by the catch
    // in the click wiring), so it never reaches here.
    setUnverifiedRestoreNotice(!result.verified);
    setRestoredId(revision.eventId);
    setSelectedId(null);
    setPendingRestoreFocus(true);
  }

  if (historyQuery.isLoading) {
    return (
      <p className="text-sm text-muted-foreground" role="status">
        Loading history...
      </p>
    );
  }

  // An initial load failure with no cached data: surface the full error state.
  // A failed background refetch (data is defined, error is also set) must not
  // unmount the history panel or clear the restore notice — show a non-destructive
  // refresh warning inside the list view instead.
  if (historyQuery.error instanceof Error && historyQuery.data === undefined) {
    return (
      <p
        className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        role="alert"
      >
        {isRelayUnreachableError(historyQuery.error)
          ? RELAY_UNREACHABLE_SHORT
          : historyQuery.error.message}
      </p>
    );
  }

  if (revisions.length === 0) {
    return (
      <p className="text-sm text-muted-foreground" role="status">
        No revisions yet.
      </p>
    );
  }

  return (
    <div className="space-y-2" data-testid="channel-canvas-history">
      {historyQuery.error instanceof Error ? (
        <p
          aria-live="polite"
          className="rounded-xl border border-border/70 bg-muted/20 px-3 py-2 text-sm text-muted-foreground"
          data-testid="channel-canvas-history-refresh-error"
          role="status"
        >
          {isRelayUnreachableError(historyQuery.error)
            ? RELAY_UNREACHABLE_SHORT
            : "Couldn't refresh history — showing last known revisions."}
        </p>
      ) : null}
      {unverifiedRestoreNotice ? (
        <p
          aria-live="polite"
          className="rounded-xl border border-border/70 bg-muted/20 px-3 py-2 text-sm text-muted-foreground"
          data-testid="channel-canvas-restore-unverified-notice"
          ref={noticeRef}
          role="status"
          tabIndex={-1}
        >
          Restored. We couldn't verify against the latest revision just now —
          check the canvas if a concurrent edit appears.
        </p>
      ) : null}
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
                disabled={restoreMutation.isPending}
                ref={revision.eventId === restoredId ? restoredRowRef : null}
                onClick={() => {
                  // Clear any prior restore error so it can't render under a
                  // different row once the selection moves — the mutation state
                  // is shared across every row.
                  restoreMutation.reset();
                  setUnverifiedRestoreNotice(false);
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
                      onClick={() =>
                        setConfirmRevision({
                          revision,
                          frozenExpectedRevision: currentRevision,
                        })
                      }
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
                    <p className="text-sm text-destructive" role="alert">
                      {canvasConflictMessage(restoreMutation.error) ??
                        restoreMutation.error.message}
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
      <AlertDialog
        onOpenChange={(open) => {
          if (!open) setConfirmRevision(null);
        }}
        open={confirmRevision !== null}
      >
        <AlertDialogContent data-testid="channel-canvas-restore-confirm">
          <AlertDialogHeader>
            <AlertDialogTitle>Restore this revision?</AlertDialogTitle>
            <AlertDialogDescription>
              This publishes{" "}
              {confirmRevision
                ? `${authorLabel(confirmRevision.revision.author)}'s revision from ${formatItemTimestamp(confirmRevision.revision.createdAt, { withTime: true })}`
                : "the selected revision"}{" "}
              as the current canvas for everyone in this channel. History is
              preserved.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button
                data-testid="channel-canvas-restore-confirm-action"
                onClick={() => {
                  const pending = confirmRevision;
                  setConfirmRevision(null);
                  if (pending) {
                    void handleRestore(
                      pending.revision,
                      pending.frozenExpectedRevision,
                    ).catch(() => {
                      // Surfaced below via restoreMutation.error.
                    });
                  }
                }}
                type="button"
              >
                Restore
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
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

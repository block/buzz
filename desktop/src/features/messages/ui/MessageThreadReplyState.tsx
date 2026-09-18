import type { ReactNode } from "react";

import {
  selectDeferredListRenderState,
  selectThreadRepliesSurface,
} from "@/features/messages/lib/timelineSnapshot";
import { Button } from "@/shared/ui/button";

/**
 * Terminal empty/error states for the thread reply region.
 *
 * These are the two non-list, non-loading outcomes of a thread-reply load. They
 * live here (rather than inline in `MessageThreadPanel`) so the load-bearing
 * distinction between them stays legible: a genuinely empty branch and a failed
 * fetch look similar but must never be confused — see `selectThreadRepliesSurface`.
 */

/**
 * A terminal load failure. This must NEVER be painted as the empty state — that
 * silently presents a broken fetch as an authoritative "no replies" and offers
 * no recovery. Any cached replies still render via the panel's "list" branch, so
 * this only surfaces when the failed load left nothing to show.
 *
 * `role="alert"` (implicit `aria-live="assertive"`, `aria-atomic="true"`) makes
 * the asynchronous failure audible to assistive tech: the card appears only after
 * the query/retry lifecycle reaches a terminal error, so without a live region a
 * screen-reader user parked in the composer never learns the load failed or that
 * Retry became available.
 */
export function ThreadRepliesErrorCard({ onRetry }: { onRetry?: () => void }) {
  return (
    <div
      className="rounded-2xl border border-dashed border-destructive/50 bg-destructive/5 px-4 py-6 text-center"
      data-testid="message-thread-replies-error"
      role="alert"
    >
      <p className="text-sm font-medium text-foreground/80">
        Couldn&apos;t load replies
      </p>
      <p className="mt-1 text-xs text-muted-foreground">
        The thread history didn&apos;t load. Check your connection and try
        again.
      </p>
      {onRetry ? (
        <Button
          className="mt-3"
          data-testid="message-thread-replies-retry"
          onClick={onRetry}
          size="sm"
          type="button"
          variant="outline"
        >
          Retry
        </Button>
      ) : null}
    </div>
  );
}

/**
 * A branch that genuinely has no replies (the load succeeded and returned none).
 * Only ever painted off the committed render state, never the raw deferred list,
 * so it can't flash while a non-empty list streams in on the deferred commit.
 */
export function ThreadRepliesEmptyCard() {
  return (
    <div className="rounded-2xl border border-dashed border-border/70 bg-card/40 px-4 py-6 text-center">
      <p className="text-sm font-medium text-foreground/80">
        No replies in this branch yet
      </p>
      <p className="mt-1 text-xs text-muted-foreground">
        Reply in the thread to continue this branch.
      </p>
    </div>
  );
}

/**
 * The single paint decision for the thread reply region. This unit owns BOTH
 * the surface selection (`selectThreadRepliesSurface`, keyed off the same raw
 * query/render state the panel already holds) AND the surface→content dispatch.
 * Fusing them here removes the last falsifiable seam: the panel passes only its
 * raw state — pending/error flags, the deferred vs. live reply counts, and the
 * huddle-transcript flag — so there is no precomputed `surface` prop at the
 * panel boundary to statically mis-set (e.g. a stray `surface="empty"` that would
 * silently restore the false-empty bug on every fetch failure). The load-bearing
 * invariant holds by construction: a terminal fetch "error" renders the retry
 * card and NEVER the "empty" "No replies" state, while "pending" paints nothing
 * (rows stream in on the deferred commit).
 *
 * The two heavy branches take render callbacks so the panel keeps ownership of
 * its skeleton and list construction (Tiptap/React-Query bound, not mountable in
 * node:test) without dragging them into this component. The mount test drives
 * the real raw-state→surface→content mapping through this exported unit, so both
 * the selection and the dispatch are covered under a cheap mount.
 */
export function ThreadReplyRegion({
  isPending,
  isError,
  deferredCount,
  liveCount,
  isHuddleTranscript = false,
  onRetry,
  renderSkeleton,
  renderList,
}: {
  isPending: boolean;
  isError: boolean;
  deferredCount: number;
  liveCount: number;
  isHuddleTranscript?: boolean;
  onRetry?: () => void;
  renderSkeleton: () => ReactNode;
  renderList: () => ReactNode;
}) {
  const surface = selectThreadRepliesSurface({
    isPending,
    isError,
    renderState: selectDeferredListRenderState(deferredCount, liveCount),
    isHuddleTranscript,
  });
  if (surface === "skeleton") return <>{renderSkeleton()}</>;
  if (surface === "list") return <>{renderList()}</>;
  if (surface === "error") return <ThreadRepliesErrorCard onRetry={onRetry} />;
  if (surface === "empty") return <ThreadRepliesEmptyCard />;
  return null;
}

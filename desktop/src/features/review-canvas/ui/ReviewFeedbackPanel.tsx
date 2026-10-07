import { truncateNpub } from "@/shared/lib/pubkey";
import type { BoundReviewFeedback, ReviewFeedbackResult } from "../hooks";
import type { ReviewDisposition, ReviewRevision } from "../lib/reviewContract";

type ReviewFeedbackPanelProps = {
  revision: ReviewRevision;
  result: ReviewFeedbackResult | undefined;
  loading: boolean;
  failed: boolean;
  currentPubkey: string | undefined;
  /**
   * Signed feedback revisions whose agent notification has not gone out,
   * taken from the durable outbox, so it also holds across a restart and while
   * the publish's acknowledgement is still unconfirmed. The relay lists such a
   * comment like any other, so it is marked here rather than shown as
   * delivered; the pending-delivery notice carries its Retry.
   */
  awaitingNotification: ReadonlySet<string>;
};

function authorLabel(pubkey: string, currentPubkey: string | undefined) {
  return currentPubkey && pubkey === currentPubkey.toLowerCase()
    ? "You"
    : truncateNpub(pubkey);
}

function DispositionBadge({
  disposition,
  revisionNumber,
}: {
  disposition: ReviewDisposition | undefined;
  revisionNumber: number;
}) {
  if (!disposition) {
    return (
      <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
        Awaiting revision
      </span>
    );
  }
  const addressed = disposition.status === "addressed";
  return (
    <span
      className={
        addressed
          ? "rounded-full bg-emerald-500/15 px-2 py-0.5 text-xs text-emerald-700 dark:text-emerald-300"
          : "rounded-full bg-amber-500/15 px-2 py-0.5 text-xs text-amber-700 dark:text-amber-300"
      }
      data-disposition={disposition.status}
    >
      {addressed ? "Addressed" : "Unresolved"} in revision {revisionNumber}
    </span>
  );
}

function FeedbackItem({
  item,
  disposition,
  revisionNumber,
  currentPubkey,
  awaitingNotification,
}: {
  item: BoundReviewFeedback;
  disposition: ReviewDisposition | undefined;
  revisionNumber: number;
  currentPubkey: string | undefined;
  awaitingNotification: boolean;
}) {
  return (
    <li
      className="flex flex-col gap-1.5 rounded-xl border border-border/60 p-2.5"
      data-testid="review-feedback-item"
    >
      <div className="flex items-center justify-between gap-2">
        {/* The block title comes from the review's own declared block table. */}
        <span className="min-w-0 truncate text-sm font-medium">
          {item.block.title}
        </span>
        {awaitingNotification ? (
          <span
            className="rounded-full bg-amber-500/15 px-2 py-0.5 text-xs text-amber-700 dark:text-amber-300"
            data-testid="review-feedback-pending"
          >
            Agent not notified yet
          </span>
        ) : (
          <DispositionBadge
            disposition={disposition}
            revisionNumber={revisionNumber}
          />
        )}
      </div>
      <p className="whitespace-pre-wrap break-words text-sm">
        {item.feedback.request}
      </p>
      <p className="text-xs text-muted-foreground">
        {item.revision.revision !== revisionNumber
          ? `On revision ${item.revision.revision} · `
          : ""}
        {authorLabel(item.feedback.authorPubkey, currentPubkey)}
      </p>
    </li>
  );
}

/**
 * Feedback attached to the open revision, and the earlier feedback this
 * revision's `feedback_dispositions` resolved. Only entries that bound to their
 * reviewed revision and a declared block are listed; the rest are counted as
 * withheld, and block labels are always the review's own.
 */
export function ReviewFeedbackPanel({
  revision,
  result,
  loading,
  failed,
  currentPubkey,
  awaitingNotification,
}: ReviewFeedbackPanelProps) {
  const dispositions = new Map(
    revision.dispositions.map((item) => [item.feedbackRevisionEventId, item]),
  );
  const items = result?.items ?? [];
  const attached = items.filter(
    (item) =>
      item.feedback.reviewed.buzzRevisionEventId === revision.eventId &&
      !dispositions.has(item.feedback.eventId),
  );
  const resolved = items.filter((item) =>
    dispositions.has(item.feedback.eventId),
  );
  const unavailable = result
    ? revision.dispositions.filter(
        (item) =>
          !items.some(
            (entry) => entry.feedback.eventId === item.feedbackRevisionEventId,
          ),
      )
    : [];

  return (
    <section aria-label="Feedback" className="flex flex-col gap-3">
      {loading ? (
        <p className="text-xs text-muted-foreground">Loading feedback…</p>
      ) : null}
      {failed ? (
        <p className="text-xs text-destructive" role="alert">
          Feedback could not be loaded.
        </p>
      ) : null}
      {result && result.withheld > 0 ? (
        <p className="text-xs text-muted-foreground">
          {result.withheld} feedback{" "}
          {result.withheld === 1 ? "entry was" : "entries were"} withheld
          because {result.withheld === 1 ? "it" : "they"} could not be verified
          against this review.
        </p>
      ) : null}
      {result?.truncated ? (
        <p
          className="text-xs text-muted-foreground"
          data-testid="review-feedback-truncated"
        >
          This revision has more feedback than can be listed here. The oldest
          comments are not shown.
        </p>
      ) : null}
      {resolved.length > 0 || unavailable.length > 0 ? (
        <div className="flex flex-col gap-1.5">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Feedback this revision responded to
          </h3>
          <ul className="flex flex-col gap-1.5">
            {resolved.map((item) => (
              <FeedbackItem
                awaitingNotification={false}
                currentPubkey={currentPubkey}
                disposition={dispositions.get(item.feedback.eventId)}
                item={item}
                key={item.feedback.eventId}
                revisionNumber={revision.revision}
              />
            ))}
            {unavailable.map((item) => (
              <li
                className="rounded-xl border border-dashed border-border/60 p-2.5 text-xs text-muted-foreground"
                data-testid="review-feedback-unavailable"
                key={item.feedbackRevisionEventId}
              >
                A feedback entry marked {item.status} is unavailable or could
                not be verified.
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      <div className="flex flex-col gap-1.5">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          Feedback on this revision
        </h3>
        {attached.length === 0 && !loading ? (
          <p className="text-xs text-muted-foreground">
            No feedback has been sent on this revision yet.
          </p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {attached.map((item) => (
              <FeedbackItem
                awaitingNotification={awaitingNotification.has(
                  item.feedback.eventId,
                )}
                currentPubkey={currentPubkey}
                disposition={undefined}
                item={item}
                key={item.feedback.eventId}
                revisionNumber={revision.revision}
              />
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

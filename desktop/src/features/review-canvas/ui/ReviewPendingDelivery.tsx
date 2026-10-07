import { Alert } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import type {
  OtherRevisionDelivery,
  PendingDelivery,
} from "../useReviewComposer";

type ReviewPendingDeliveryProps = {
  /** Unfinished comments signed against the open revision. */
  deliveries: PendingDelivery[];
  /** Unfinished comments signed against another revision of this artifact. */
  otherRevisionDeliveries: OtherRevisionDelivery[];
  /** Open the comment form of a block; its locked form offers Retry. */
  onOpen: (blockId: string) => void;
  /** Go to the exact revision a comment was signed against to finish it. */
  onOpenRevision: (revisionEventId: string) => void;
};

/** What is true of one unfinished comment, in the reviewer's terms. */
function describe(delivery: PendingDelivery): string {
  switch (delivery.publishState) {
    case "accepted":
      return "your feedback was published, but the agent has not been notified.";
    case "ambiguous":
      return "your comment was sent, but the relay has not confirmed it. It cannot be discarded until it does; the agent is notified once it is.";
    case "rejected":
      return "the relay refused your comment. Retry it, or discard it.";
    case "never_attempted":
      return "your comment is signed, but it has not been sent yet.";
    default:
      return "your comment is signed, but the relay has not confirmed it yet.";
  }
}

/**
 * Signed comments that have not finished, listed so they are found again
 * after leaving and reopening the review, or restarting Buzz: a comment the
 * relay may already hold must never look delivered while the agent was not
 * notified. A comment on the open revision opens the same locked form (Retry,
 * and Discard only where the relay provably holds nothing) that the block list
 * reaches. A comment signed against another revision belongs to that revision:
 * it names it and navigates there, and is never opened as the open
 * revision's own.
 */
export function ReviewPendingDelivery({
  deliveries,
  otherRevisionDeliveries,
  onOpen,
  onOpenRevision,
}: ReviewPendingDeliveryProps) {
  if (deliveries.length === 0 && otherRevisionDeliveries.length === 0) {
    return null;
  }
  return (
    <Alert data-testid="review-pending-delivery" role="status">
      <p className="font-medium">Feedback waiting to finish</p>
      <ul className="mt-2 flex flex-col gap-2">
        {deliveries.map((delivery) => (
          <li
            className="flex flex-col items-start gap-1"
            key={delivery.blockId}
          >
            <p>
              On “{delivery.blockTitle}”: {describe(delivery)}
            </p>
            <Button
              data-testid={`review-pending-open-${delivery.blockId}`}
              onClick={() => onOpen(delivery.blockId)}
              size="sm"
              type="button"
              variant="outline"
            >
              Finish feedback on “{delivery.blockTitle}”
            </Button>
          </li>
        ))}
        {otherRevisionDeliveries.map((delivery) => (
          <li
            className="flex flex-col items-start gap-1"
            key={`${delivery.revisionEventId}:${delivery.blockId}`}
          >
            <p>
              On “{delivery.blockTitle}” in revision {delivery.revisionNumber}:{" "}
              {describe(delivery)}
            </p>
            <Button
              data-testid={`review-pending-revision-${delivery.revisionEventId}-${delivery.blockId}`}
              onClick={() => onOpenRevision(delivery.revisionEventId)}
              size="sm"
              type="button"
              variant="outline"
            >
              {`Go to revision ${delivery.revisionNumber} to finish “${delivery.blockTitle}”`}
            </Button>
          </li>
        ))}
      </ul>
    </Alert>
  );
}

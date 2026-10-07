import {
  isHex64,
  type ReviewNotification,
  type ReviewRevision,
  ReviewContractError,
} from "./reviewContract";

/**
 * Context a Review Canvas opens with. Every field is an untrusted URL value
 * that is only ever *compared* against the verified revision or used as a
 * Nostr reference, never rendered as markup.
 */
export type ReviewRouteSearch = {
  /** Exact review revision (NIP-AR event ID) to open. */
  revision?: string;
  /** Executive agent feedback must wake. */
  agent?: string;
  /** Payload digest the notification promised for `revision`. */
  digest?: string;
  /** Signer of the notification; the revision must share it. */
  author?: string;
  /** The kind-9 notification message the review was opened from. */
  notification?: string;
  /** Thread root the notification lives in. */
  thread?: string;
};

const SEARCH_KEYS = [
  "revision",
  "agent",
  "digest",
  "author",
  "notification",
  "thread",
] as const;

/** Keep only well-formed 64-hex values from a raw URL search object. */
export function validateReviewSearch(
  search: Record<string, unknown>,
): ReviewRouteSearch {
  const result: ReviewRouteSearch = {};
  for (const key of SEARCH_KEYS) {
    const value = search[key];
    if (isHex64(value)) result[key] = value;
  }
  return result;
}

/** The route context a notification card opens its review with. */
export function reviewSearchFromNotification(
  notification: ReviewNotification,
): ReviewRouteSearch {
  return {
    revision: notification.revisionEventId,
    agent: notification.executiveAgentPubkey,
    digest: notification.payloadDigest,
    ...(notification.authorPubkey ? { author: notification.authorPubkey } : {}),
    notification: notification.messageId,
    thread: notification.threadRootId,
  };
}

/**
 * Check a loaded revision against what its notification announced. A review
 * that was swapped, re-signed by someone else, or republished with different
 * bytes must never render as the thing the reviewer was told they were opening.
 */
export function verifyAnnouncement(
  revision: ReviewRevision,
  announced: Pick<ReviewRouteSearch, "author" | "digest">,
): void {
  if (announced.author && revision.authorPubkey !== announced.author) {
    throw new ReviewContractError(
      "This review was not published by the account that announced it, so it was not opened.",
    );
  }
  if (
    announced.digest &&
    revision.synaxisArtifact.payloadDigest !== announced.digest
  ) {
    throw new ReviewContractError(
      "This review no longer matches its notification: the payload digest changed.",
    );
  }
}

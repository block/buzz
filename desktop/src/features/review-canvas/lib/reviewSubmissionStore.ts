import {
  type ReviewFeedbackDeps,
  ReviewFeedbackSubmission,
} from "./reviewFeedbackSubmission";
import {
  identityKey,
  identityOf,
  type ReviewOutboxScope,
  type ReviewSubmissionIdentity,
} from "./reviewOutbox";

/**
 * Live handles on unfinished feedback submissions, over the durable outbox.
 *
 * A signed submission is the only record that lets a reviewer finish a comment
 * the relay may already hold: republishing its identical event and sending its
 * wake. The workbench that created it can unmount at any time (back to the
 * thread, "Open latest", a route change) and the app can quit, so the comment
 * is not component state. Its source of truth is the outbox on disk
 * (`reviewOutbox`), written by the submission itself before anything is sent;
 * this map only keeps the one live object per identity so every workbench and
 * every in-flight run share it.
 *
 * Nothing secret is held or written: a signed event and the comment text it
 * carries are what the relay already stores.
 */

export type { ReviewSubmissionIdentity } from "./reviewOutbox";

const submissions = new Map<string, ReviewFeedbackSubmission>();

/** The identity a submission captured; it never changes after construction. */
export function submissionIdentity(
  submission: ReviewFeedbackSubmission,
): ReviewSubmissionIdentity {
  return identityOf(submission.context);
}

export function rememberSubmission(submission: ReviewFeedbackSubmission) {
  submissions.set(identityKey(submissionIdentity(submission)), submission);
}

/**
 * The submission for one exact relay, signer, channel, artifact, reviewed
 * revision, and block: the live one if this process has it, else the one
 * restored from the outbox (after a restart or a store reset).
 */
export function findSubmission(
  identity: ReviewSubmissionIdentity,
  deps: ReviewFeedbackDeps,
): ReviewFeedbackSubmission | undefined {
  const key = identityKey(identity);
  const live = submissions.get(key);
  if (live) return live;
  const entry = deps.outbox.load(identity);
  if (!entry) return undefined;
  const restored = ReviewFeedbackSubmission.restore(entry, deps);
  submissions.set(key, restored);
  return restored;
}

/**
 * Every unfinished signed comment on one reviewed artifact, across its
 * revisions: the comments whose relay listing must not read as delivered.
 */
export function listUnfinished(
  scope: ReviewOutboxScope,
  deps: ReviewFeedbackDeps,
): ReviewFeedbackSubmission[] {
  const unfinished: ReviewFeedbackSubmission[] = [];
  for (const entry of deps.outbox.list(scope)) {
    const submission = findSubmission(identityOf(entry.context), deps);
    if (submission?.frozen && !submission.completed) {
      unfinished.push(submission);
    }
  }
  return unfinished;
}

/**
 * Drop the live handle, but only if it is still the one remembered for its
 * identity, so a stale handle can never evict a newer submission. The outbox
 * is untouched: the submission clears it itself once it finished or was
 * discarded.
 */
export function forgetSubmission(submission: ReviewFeedbackSubmission) {
  const key = identityKey(submissionIdentity(submission));
  if (submissions.get(key) === submission) submissions.delete(key);
}

/**
 * Forget every live handle, as quitting and restarting the app does. The
 * outbox on disk is deliberately kept, so unfinished comments are restored on
 * the next lookup. Test isolation and restart simulation only.
 */
export function resetSubmissionStore() {
  submissions.clear();
}

import { isDefinitiveRelayRefusal } from "@/shared/api/relayEventRejection";
import type { RelayEvent } from "@/shared/api/types";
import {
  buildFeedbackArtifact,
  type FeedbackArtifactTemplate,
  type FeedbackViewport,
  isHex64,
  normalizeFeedbackRequest,
  type ReviewBlock,
  ReviewContractError,
  type ReviewRevision,
} from "./reviewContract";
import {
  type FeedbackPublishState,
  identityOf,
  type OutboxEntry,
  type OutboxRecord,
  type ReviewOutbox,
} from "./reviewOutbox";

/** Everything that identifies one feedback submission; fixed for its lifetime. */
export type ReviewFeedbackContext = {
  revision: ReviewRevision;
  block: ReviewBlock;
  request: string;
  viewport: FeedbackViewport;
  /** Executive agent the wake message must mention. */
  executiveAgentPubkey: string;
  /** Thread the wake message replies into. */
  parentEventId: string;
  rootEventId: string;
  /** Tenant scope captured when the reviewer pressed submit: both required. */
  expectedRelayUrl: string;
  expectedSignerPubkey: string;
};

export type ReviewWakeInput = {
  channelId: string;
  content: string;
  parentEventId: string;
  rootEventId: string;
  executiveAgentPubkey: string;
  feedbackArtifactId: string;
  feedbackRevisionEventId: string;
  reviewedArtifactId: string;
  reviewedRevisionEventId: string;
  /**
   * Frozen with the draft at the first wake attempt, so a prompt retry rebuilds
   * the byte-identical event. The relay refuses a timestamp more than 15
   * minutes off; the native command then retries once with a fresh one, and the
   * relay's feedback-wake ledger (keyed by author and feedback revision)
   * guarantees that still stores at most one wake.
   */
  createdAt: number;
  expectedRelayUrl: string;
  expectedSignerPubkey: string;
};

/** The exact event a relay read must find, and the scope it must be read in. */
export type FeedbackReconcileInput = {
  event: RelayEvent;
  channelId: string;
  feedbackArtifactId: string;
  expectedRelayUrl: string;
  expectedSignerPubkey: string;
};

/** What the authoritative relay read found for one exact feedback event. */
export type FeedbackReconcileResult = "held" | "absent";

export type ReviewFeedbackDeps = {
  newFeedbackId: () => string;
  nowSeconds: () => number;
  signEvent: (template: FeedbackArtifactTemplate) => Promise<RelayEvent>;
  /**
   * Publish; `isCurrent` is checked immediately before every send/retry. An
   * explicit relay refusal must reject with `RelayEventRejectedError`; every
   * other rejection is treated as "the relay may hold it".
   */
  publishEvent: (
    event: RelayEvent,
    isCurrent: () => boolean,
  ) => Promise<unknown>;
  sendNotification: (input: ReviewWakeInput) => Promise<unknown>;
  /** True while the active community and account still match the capture. */
  isScopeCurrent: (context: ReviewFeedbackContext) => boolean;
  /**
   * Rejects unless the reviewed revision is, right now, the review head. Called
   * only before a new event is signed, never to gate a frozen event or wake.
   */
  assertHeadCurrent: (revision: ReviewRevision) => Promise<void>;
  /**
   * Ask the authoritative relay whether it holds exactly this event. Rejects
   * when it cannot answer; "absent" is an answer.
   */
  reconcileFeedback: (
    input: FeedbackReconcileInput,
  ) => Promise<FeedbackReconcileResult>;
  /** Durable, bounded store of every unfinished signed comment. */
  outbox: ReviewOutbox;
};

export type ReviewFeedbackReceipt = {
  feedbackEventId: string;
  feedbackArtifactId: string;
};

/** `held`: the relay stores the comment, so it was kept and cannot be dropped. */
export type DiscardOutcome = "discarded" | "held";

const SCOPE_CHANGED =
  "The active community or account changed, so nothing more was sent. Reopen the review there to retry.";
const BUSY =
  "Another action on this comment is still running. Wait for it to finish.";
const NOT_DISCARDABLE =
  "This comment may already be on the relay, so it cannot be discarded. Retry to finish it.";
const NOT_CONFIRMED =
  "Buzz could not confirm with the relay that this comment was never stored, so it was kept. Try again when the relay is reachable.";

function sameTags(left: string[][], right: string[][]) {
  return JSON.stringify(left) === JSON.stringify(right);
}

/** A one-line, bounded excerpt for the human-readable wake message. */
function excerpt(request: string, limit = 160): string {
  const line = request.replace(/\s+/g, " ").trim();
  return line.length <= limit ? line : `${line.slice(0, limit - 1)}…`;
}

/**
 * One reviewer comment on one block, published as an immutable feedback
 * revision and then announced to the executive agent.
 *
 * Three things are frozen and reused by every retry:
 * - the signed feedback event, so a lost acknowledgement republishes the
 *   identical event (the relay accepts it once) instead of forking a second
 *   revision;
 * - the wake's inputs, so the native layer rebuilds the same kind-9 event; and
 * - the outbox record of both, written to disk *before* anything is sent, so
 *   quitting at any point leaves a comment that can be finished.
 *
 * What is known about the relay holding the signed event is an explicit
 * {@link FeedbackPublishState}, never inferred from a promise: a publish that
 * ends without the relay's answer is `ambiguous` (the relay may have stored it
 * and lost only the acknowledgement), and only an explicit refusal makes it
 * `rejected`. The state is written ahead of the send (`ambiguous`), so even a
 * process that dies mid-send resumes as ambiguous, never as "never sent".
 *
 * Before any resend of an event whose fate is unknown, the relay is read for
 * that exact event: if it holds it, it is accepted and only the wake remains.
 * A comment may be discarded only while the relay provably holds nothing:
 * never sent, or explicitly refused *and* confirmed absent by a fresh read.
 * An ambiguous comment is never discardable and never replaceable.
 *
 * Before each signature, publish, and wake the captured community and account
 * must still be active. Before the signature the reviewed revision must still
 * be the review head (the relay enforces the same rule atomically; this only
 * avoids a doomed signature). Once signed, no head check gates the event: a
 * publish or retry reaches the relay regardless of the head, which returns
 * success for an event it already holds and a stale conflict for one it never
 * accepted. The wake is attempted only after the relay accepted the feedback,
 * and the record is cleared only after the wake succeeded.
 * Concurrent `submit()` calls share one run.
 */
export class ReviewFeedbackSubmission {
  readonly context: ReviewFeedbackContext;
  readonly request: string;
  readonly feedbackArtifactId: string;
  private readonly deps: ReviewFeedbackDeps;
  private signed: RelayEvent | null = null;
  private publish: FeedbackPublishState = "never_attempted";
  private wake: ReviewWakeInput | null = null;
  private notified = false;
  private discarded = false;
  private inFlight: Promise<ReviewFeedbackReceipt> | null = null;
  private maintenance: Promise<unknown> | null = null;

  constructor(
    context: ReviewFeedbackContext,
    deps: ReviewFeedbackDeps,
    feedbackArtifactId?: string,
  ) {
    this.context = context;
    this.deps = deps;
    // Validate before anything is signed; a bad comment never reaches the signer.
    this.request = normalizeFeedbackRequest(context.request);
    if (!isHex64(context.executiveAgentPubkey)) {
      throw new ReviewContractError("The executive agent is not identifiable.");
    }
    if (!context.expectedRelayUrl || !isHex64(context.expectedSignerPubkey)) {
      throw new ReviewContractError(
        "The active community and account are not ready, so feedback cannot be sent.",
      );
    }
    this.feedbackArtifactId = feedbackArtifactId ?? deps.newFeedbackId();
  }

  /**
   * Rebuild a submission from its durable record after a restart or remount,
   * exactly as it was left: same signed event, same publish state, same wake.
   */
  static restore(
    entry: OutboxEntry,
    deps: ReviewFeedbackDeps,
  ): ReviewFeedbackSubmission {
    const submission = new ReviewFeedbackSubmission(
      entry.context,
      deps,
      entry.feedbackArtifactId,
    );
    submission.signed = entry.signed;
    submission.publish = entry.publish;
    submission.wake = entry.wake;
    return submission;
  }

  /** True once a signed payload exists; the comment can no longer change. */
  get frozen(): boolean {
    return this.signed !== null && !this.discarded;
  }

  /** What is known about the relay holding the signed feedback event. */
  get publishState(): FeedbackPublishState {
    return this.publish;
  }

  /** True once the relay is known to hold the feedback revision. */
  get feedbackPublished(): boolean {
    return this.publish === "accepted";
  }

  get completed(): boolean {
    return this.notified;
  }

  /** True while any action on this comment is in flight, whoever started it. */
  get running(): boolean {
    return this.inFlight !== null || this.maintenance !== null;
  }

  /**
   * The frozen feedback revision's event ID, once signed. Present whenever the
   * relay *might* hold it, so a listed comment can be matched to its owed wake.
   */
  get feedbackEventId(): string | null {
    return this.frozen ? (this.signed?.id ?? null) : null;
  }

  /**
   * Whether abandoning is safe right now: the relay provably holds nothing.
   * Never true for an ambiguous or accepted comment. A background relay read
   * does not change the answer (it only ever moves a comment toward
   * accepted), so the answer does not flicker while one runs.
   */
  get canDiscard(): boolean {
    return (
      this.frozen &&
      !this.notified &&
      this.inFlight === null &&
      (this.publish === "never_attempted" || this.publish === "rejected")
    );
  }

  submit(): Promise<ReviewFeedbackReceipt> {
    if (this.inFlight) return this.inFlight;
    const maintenance = this.maintenance;
    const run = (async () => {
      // A background relay read may be settling this very comment; its answer
      // decides what is left to do, so wait for it instead of racing it.
      if (maintenance) await maintenance.catch(() => undefined);
      if (this.discarded) {
        throw new ReviewContractError("This comment was discarded.");
      }
      return this.run();
    })().finally(() => {
      this.inFlight = null;
    });
    this.inFlight = run;
    return run;
  }

  /**
   * Ask the relay whether it already holds a comment whose publish ended
   * without an answer, and adopt it as accepted if so. Best effort: it never
   * throws, and an unreachable relay leaves the state unchanged.
   */
  async reconcile(): Promise<void> {
    const event = this.signed;
    const undecided =
      this.publish === "ambiguous" || this.publish === "rejected";
    if (!event || this.discarded || !undecided || this.running) return;
    try {
      await this.exclusive(async () => {
        if (!this.deps.isScopeCurrent(this.context)) return;
        if (await this.relayHolds(event)) this.transition("accepted", false);
      });
    } catch {
      // Unreachable or busy: the state stays as it was.
    }
  }

  /**
   * Abandon the comment, but only when the relay provably holds nothing. A
   * never-sent comment is dropped. An explicitly refused one is first read
   * back from the relay: if the relay holds it after all, it is kept as
   * accepted (`held`) and its wake is still owed.
   */
  async discard(): Promise<DiscardOutcome> {
    if (this.inFlight) throw new ReviewContractError(BUSY);
    // A background relay read may be settling this very comment (it can
    // adopt it as accepted); its answer decides whether discarding is allowed.
    if (this.maintenance) await this.maintenance.catch(() => undefined);
    if (!this.canDiscard) throw new ReviewContractError(NOT_DISCARDABLE);
    return this.exclusive<DiscardOutcome>(async () => {
      const event = this.signed as RelayEvent;
      if (this.publish === "rejected") {
        this.assertScope();
        let held: boolean;
        try {
          held = await this.relayHoldsOrThrow(event);
        } catch {
          throw new ReviewContractError(NOT_CONFIRMED);
        }
        if (held) {
          this.transition("accepted", false);
          return "held";
        }
      }
      this.discarded = true;
      try {
        this.deps.outbox.remove(identityOf(this.context));
      } catch {
        // A leftover never-stored entry is harmless: it reloads as
        // discardable and expires.
      }
      return "discarded";
    });
  }

  private async exclusive<T>(operation: () => Promise<T>): Promise<T> {
    if (this.running) throw new ReviewContractError(BUSY);
    const run = operation();
    this.maintenance = run;
    try {
      return await run;
    } finally {
      this.maintenance = null;
    }
  }

  private assertScope() {
    if (!this.deps.isScopeCurrent(this.context)) {
      throw new ReviewContractError(SCOPE_CHANGED);
    }
  }

  private record(): OutboxRecord {
    return {
      context: { ...this.context, request: this.request },
      feedbackArtifactId: this.feedbackArtifactId,
      signed: this.signed as RelayEvent,
      publish: this.publish,
      wake: this.wake,
    };
  }

  /**
   * Record a new publish state. A `durable` change must reach the outbox
   * before the caller acts on it; if it cannot, the state reverts and the
   * error stops the caller. Others are best effort: the stored record then
   * lags in the safe direction (`ambiguous` until the next reconcile).
   */
  private transition(next: FeedbackPublishState, durable: boolean) {
    const previous = this.publish;
    this.publish = next;
    try {
      this.deps.outbox.save(this.record());
    } catch (error) {
      if (durable) {
        this.publish = previous;
        throw error;
      }
    }
  }

  private relayHoldsOrThrow(event: RelayEvent): Promise<boolean> {
    const { context } = this;
    return this.deps
      .reconcileFeedback({
        event,
        channelId: context.revision.channelId,
        feedbackArtifactId: this.feedbackArtifactId,
        expectedRelayUrl: context.expectedRelayUrl,
        expectedSignerPubkey: context.expectedSignerPubkey,
      })
      .then((found) => found === "held");
  }

  /** A failed read is "not known to hold it", never proof that it does. */
  private async relayHolds(event: RelayEvent): Promise<boolean> {
    try {
      return await this.relayHoldsOrThrow(event);
    } catch {
      return false;
    }
  }

  private async sign(): Promise<RelayEvent> {
    const { context, deps } = this;
    this.assertScope();
    deps.outbox.assertRoom(identityOf(context), this.feedbackArtifactId);
    await deps.assertHeadCurrent(context.revision);
    const template = buildFeedbackArtifact({
      feedbackArtifactId: this.feedbackArtifactId,
      revision: context.revision,
      block: context.block,
      request: this.request,
      viewport: context.viewport,
    });
    const signed = await deps.signEvent(template);
    const signerOk =
      signed.pubkey.toLowerCase() ===
      context.expectedSignerPubkey.toLowerCase();
    if (
      !isHex64(signed.id) ||
      signed.kind !== template.kind ||
      signed.content !== template.content ||
      !sameTags(signed.tags, template.tags) ||
      !signerOk
    ) {
      throw new ReviewContractError(
        "The signed feedback did not match what was composed; nothing was sent.",
      );
    }
    // Durable before anything is sent: from here a restart resumes this exact
    // event. If it cannot be saved, it is not kept and nothing is sent.
    this.signed = signed;
    try {
      deps.outbox.save(this.record());
    } catch (error) {
      this.signed = null;
      throw error;
    }
    return signed;
  }

  private async publishFrozen(event: RelayEvent) {
    const { context, deps } = this;
    this.assertScope();
    // A previous attempt may have committed: read the relay before resending.
    if (this.publish !== "never_attempted" && (await this.relayHolds(event))) {
      this.transition("accepted", false);
      return;
    }
    // Reading awaited: the scope may have changed since it was checked. The
    // head is deliberately not re-read: this may be a retry after a lost
    // acknowledgement, which only the relay can reconcile.
    this.assertScope();
    // Write-ahead: from here the relay may receive the event whatever happens
    // next, even if this process never sees the outcome.
    this.transition("ambiguous", true);
    try {
      await deps.publishEvent(event, () => deps.isScopeCurrent(context));
    } catch (error) {
      if (isDefinitiveRelayRefusal(error)) this.transition("rejected", false);
      throw error;
    }
    this.transition("accepted", false);
  }

  private async notifyAgent(event: RelayEvent) {
    const { context, deps } = this;
    this.assertScope();
    if (!this.wake) {
      this.wake = {
        channelId: context.revision.channelId,
        content: `Feedback on “${context.block.title}” (revision ${context.revision.revision}): ${excerpt(this.request)}`,
        parentEventId: context.parentEventId,
        rootEventId: context.rootEventId,
        executiveAgentPubkey: context.executiveAgentPubkey,
        feedbackArtifactId: this.feedbackArtifactId,
        feedbackRevisionEventId: event.id,
        reviewedArtifactId: context.revision.artifactId,
        reviewedRevisionEventId: context.revision.eventId,
        createdAt: deps.nowSeconds(),
        expectedRelayUrl: context.expectedRelayUrl,
        expectedSignerPubkey: context.expectedSignerPubkey,
      };
      try {
        deps.outbox.save(this.record());
      } catch {
        // The relay deduplicates the wake by feedback revision, so a lost
        // timestamp only means rebuilding it; the comment itself is saved.
      }
    }
    await deps.sendNotification(this.wake);
    this.notified = true;
    try {
      deps.outbox.remove(identityOf(context));
    } catch {
      // Left behind, it reloads as accepted; the next wake is deduplicated by
      // the relay and clears it.
    }
  }

  private async run(): Promise<ReviewFeedbackReceipt> {
    const event = this.signed ?? (await this.sign());
    if (this.publish !== "accepted") await this.publishFrozen(event);
    if (!this.notified) await this.notifyAgent(event);
    return {
      feedbackEventId: event.id,
      feedbackArtifactId: this.feedbackArtifactId,
    };
  }
}

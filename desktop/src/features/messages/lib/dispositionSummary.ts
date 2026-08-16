import type { MainTimelineEntry } from "./threadPanel";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Channel accountability counts.
 *
 * Five non-overloaded buckets. An earlier version had an `answered` count
 * that incremented for *any* disposition, including `errored`, so a channel
 * whose every turn failed could render "all requests answered". "Has some
 * record" and "was answered" are different facts, and a summary that blurs
 * them is worse than no summary.
 *
 * Every marked request lands in exactly one bucket, so `total` is their sum
 * and nothing is counted twice.
 */
export type ChannelDispositionSummary = {
  /** Marked requests among currently-loaded messages. */
  total: number;
  /** Settled: a terminal claim bound. The only "done" count. */
  settled: number;
  /** Answered, not settled (`responded`). */
  respondedUnsettled: number;
  /** The turn failed (`errored`). Not an answer. */
  attemptFailed: number;
  /** Valid obligations with nothing bound: real gaps. */
  noRecord: Array<{ id: string; body: string }>;
  /** Contradictory terminal claims. */
  disputed: number;
  /**
   * Marked events that are not usable obligations — malformed (a client bug)
   * or unrepresentable in v1. Never counted against an agent, and never
   * folded into `noRecord`: filing a client fault as an agent's gap creates a
   * gap nobody can ever clear.
   */
  invalidOrUnsupported: number;
  /** Anything not settled: what a reader should look at. */
  needsAttention: number;
};

/**
 * Whether the summary has anything to render.
 *
 * The chip's own early return and its caller's layout wrapper must agree
 * exactly. When they were written separately they did not: the chip returned
 * `null` for an empty channel while the wrapper still rendered its `pb-1.5`,
 * leaving 6px of dead space under the header of every channel with no marked
 * requests — which is nearly all of them — and knocking the timeline's sticky
 * day divider off its designed 8px offset.
 *
 * Exported so there is one definition with two callers rather than one
 * condition written twice.
 */
export function hasVisibleRequests(
  summary: ChannelDispositionSummary,
): boolean {
  return summary.total > 0;
}

/**
 * Derives the channel accountability summary from whatever timeline data is
 * currently loaded.
 *
 * **This tallies; it does not derive.** Every `requestStatus` was produced by
 * the shared verifier in `formatTimelineMessages`, so there is no second
 * classification here that could disagree with the CLI's. The previous
 * version *did* classify independently — it treated any marked message
 * without a disposition as unanswered — and that put invalid requests in the
 * agent-gap bucket, contradicting the rule stated in the file next to it.
 * `dispositionSummary.adapter.test.mjs` pins this tally against the shared
 * `account()` over the same events.
 *
 * The counts reflect loaded/visible history, not necessarily the channel's
 * full lifetime: a request that scrolled out of the loaded window before its
 * disposition was aux-hydrated could be undercounted. That is why no surface
 * built on this may make a channel-wide claim — see `Coverage` in the shared
 * module, and design.md Decision 5.
 */
export function deriveChannelDispositionSummary(
  entries: readonly MainTimelineEntry[],
): ChannelDispositionSummary {
  const summary: ChannelDispositionSummary = {
    total: 0,
    settled: 0,
    respondedUnsettled: 0,
    attemptFailed: 0,
    noRecord: [],
    disputed: 0,
    invalidOrUnsupported: 0,
    needsAttention: 0,
  };

  for (const { message } of entries) {
    const status: TimelineMessage["requestStatus"] = message.requestStatus;
    if (!status) {
      continue;
    }
    summary.total += 1;

    if (status.kind === "invalid" || status.kind === "unsupported") {
      summary.invalidOrUnsupported += 1;
      summary.needsAttention += 1;
      continue;
    }

    switch (status.outcome.kind) {
      case "settled":
        summary.settled += 1;
        break;
      case "open":
        if (status.outcome.state === "errored") {
          summary.attemptFailed += 1;
        } else {
          summary.respondedUnsettled += 1;
        }
        summary.needsAttention += 1;
        break;
      case "unanswered":
        summary.noRecord.push({ id: message.id, body: message.body });
        summary.needsAttention += 1;
        break;
      case "disputed":
        summary.disputed += 1;
        summary.needsAttention += 1;
        break;
    }
  }

  return summary;
}

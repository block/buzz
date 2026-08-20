import { CheckCircle2, ChevronDown } from "lucide-react";

import {
  type ChannelDispositionSummary,
  hasVisibleRequests,
} from "@/features/messages/lib/dispositionSummary";
import { Badge } from "@/shared/ui/badge";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

/**
 * Channel accountability summary — "N requests answered" derived from
 * currently-loaded timeline data (see dispositionSummary.ts for the scope
 * caveat, surfaced below in the popover too). Expands to a list of
 * unanswered requests with jump-to-message. Placement is a visual decision
 * left to PR review (see design.md's Open Questions) — this component only
 * needs a spot near the timeline.
 *
 * The green "all resolved" treatment requires more than
 * `unanswered.length === 0`: `errored` is non-terminal (NIP-AD.md's
 * lifecycle calls it open/repairable, not resolved) and a `conflict` is an
 * unresolved contradiction by definition, so both must be zero too. A
 * channel with every request answered but one still `errored` or
 * conflicted is not "all good" and must not render as if it were.
 */
export function DispositionSummaryChip({
  summary,
  onJumpToMessage,
}: {
  summary: ChannelDispositionSummary;
  onJumpToMessage: (messageId: string) => void;
}) {
  if (!hasVisibleRequests(summary)) {
    return null;
  }

  const needsAttention = summary.needsAttention;
  const allSettled = needsAttention === 0;

  // "visible", never "all". This summary derives from the currently loaded
  // timeline window, which is not provably the channel's whole history — the
  // shared verifier calls that incomplete `Coverage`, and a surface that says
  // "all requests" from a partial window makes exactly the channel-wide claim
  // it cannot support. The qualifier is in the chip itself, not only in the
  // popover, because the chip is what gets read.
  //
  // There is deliberately no "all answered" state. An earlier version had
  // one, computed from `unanswered.length === 0` against a count that
  // incremented for any disposition including `errored` — so a channel whose
  // every turn failed rendered "All visible requests answered". Settled is
  // the only claim worth making, and everything else is "needs attention".
  const headerText = allSettled
    ? "All visible requests settled"
    : `${needsAttention} of ${summary.total} need${needsAttention === 1 ? "s" : ""} attention`;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Badge
          variant={allSettled ? "success" : "outline"}
          className="inline-flex cursor-pointer items-center gap-1 normal-case tracking-normal"
        >
          {allSettled ? (
            <CheckCircle2 className="size-3" aria-hidden="true" />
          ) : null}
          {summary.total} visible request{summary.total === 1 ? "" : "s"} ·{" "}
          {summary.settled} settled
          {needsAttention > 0 ? ` · ${needsAttention} need attention` : ""}
          <ChevronDown className="size-3" aria-hidden="true" />
        </Badge>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 text-sm">
        <p className="font-medium">{headerText}</p>
        <p className="mt-1 text-2xs text-muted-foreground">
          Counts only messages loaded in this view, not the channel's full
          history.
        </p>
        {/* Five non-overloaded counts. Each request appears in exactly one. */}
        <dl className="mt-2 space-y-1 text-2xs text-muted-foreground">
          <div className="flex justify-between">
            <dt>Settled</dt>
            <dd>{summary.settled}</dd>
          </div>
          <div className="flex justify-between">
            <dt>Responded (not settled)</dt>
            <dd>{summary.respondedUnsettled}</dd>
          </div>
          <div className="flex justify-between">
            <dt>Attempt failed</dt>
            <dd>{summary.attemptFailed}</dd>
          </div>
          <div className="flex justify-between">
            <dt>No record</dt>
            <dd>{summary.noRecord.length}</dd>
          </div>
          <div className="flex justify-between">
            <dt>Disputed</dt>
            <dd>{summary.disputed}</dd>
          </div>
          {summary.invalidOrUnsupported > 0 ? (
            <div className="flex justify-between">
              {/* Attributed to the sender, never to an agent. */}
              <dt>Invalid or not tracked</dt>
              <dd>{summary.invalidOrUnsupported}</dd>
            </div>
          ) : null}
        </dl>
        {summary.noRecord.length > 0 ? (
          <ul className="mt-3 space-y-1 border-t border-border/60 pt-2">
            {summary.noRecord.map((request) => (
              <li key={request.id}>
                <button
                  type="button"
                  className="w-full truncate text-left text-2xs text-muted-foreground hover:text-foreground"
                  onClick={() => onJumpToMessage(request.id)}
                >
                  {request.body || "(empty message)"}
                </button>
              </li>
            ))}
          </ul>
        ) : null}
      </PopoverContent>
    </Popover>
  );
}

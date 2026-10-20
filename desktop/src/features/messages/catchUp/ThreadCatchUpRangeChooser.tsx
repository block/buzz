import { Button } from "@/shared/ui/button";
import {
  type CatchUpBaseline,
  type CatchUpRangeChoice,
  formatCatchUpTime,
} from "./threadCatchUp";

function describeBaseline(baseline: CatchUpBaseline): string {
  switch (baseline.kind) {
    case "marker":
      return `${baseline.unreadCount} new ${baseline.unreadCount === 1 ? "reply" : "replies"} since you last read this thread (${formatCatchUpTime(baseline.previousReadAt)}).`;
    case "caught-up":
      return `No new replies since you last read this thread (${formatCatchUpTime(baseline.previousReadAt)}). You can still recap a recent period.`;
    case "expired":
      return "Your last read position here is older than 7 days, so it can't mark where to start. Choose a period to recap.";
    case "absent":
      return "There's no earlier read position for this thread. Choose a period to recap.";
    case "unresolved":
      return "Your read position couldn't be confirmed for this visit. Choose a period to recap.";
  }
}

/**
 * Picks the recap period. Only a confirmed previous read position is offered
 * as "since you last read"; anything uncertain requires an explicit lookback.
 * Generating changes nothing about what is read.
 */
export function ThreadCatchUpRangeChooser({
  baseline,
  model,
  onChangeSetup,
  onGenerate,
}: {
  baseline: CatchUpBaseline;
  model: string;
  onChangeSetup: () => void;
  onGenerate: (choice: CatchUpRangeChoice) => void;
}) {
  const hasMarker = baseline.kind === "marker";
  return (
    <div className="space-y-3" data-testid="thread-catch-up-range">
      <p
        className="text-sm leading-5"
        data-baseline={baseline.kind}
        data-testid="thread-catch-up-baseline"
      >
        {describeBaseline(baseline)}
      </p>
      <fieldset className="flex min-w-0 flex-wrap items-center gap-2 border-0 p-0">
        <legend className="sr-only">Recap period</legend>
        {hasMarker ? (
          <Button
            data-testid="thread-catch-up-generate"
            onClick={() => onGenerate({ kind: "baseline" })}
            size="sm"
            type="button"
          >
            Recap new replies
          </Button>
        ) : null}
        <Button
          data-testid="thread-catch-up-lookback-day"
          onClick={() => onGenerate({ kind: "lookback", lookback: "day" })}
          size="sm"
          type="button"
          variant="outline"
        >
          Last 24 hours
        </Button>
        <Button
          data-testid="thread-catch-up-lookback-week"
          onClick={() => onGenerate({ kind: "lookback", lookback: "week" })}
          size="sm"
          type="button"
          variant="outline"
        >
          Last 7 days
        </Button>
      </fieldset>
      <p className="text-2xs text-muted-foreground">
        Private to you until you share it. Uses {model} on this computer.{" "}
        <Button
          className="h-auto p-0 text-2xs"
          data-testid="thread-catch-up-change-setup"
          onClick={onChangeSetup}
          type="button"
          variant="link"
        >
          Change setup
        </Button>
      </p>
    </div>
  );
}

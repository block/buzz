import {
  AlertTriangle,
  CheckCircle2,
  CircleSlash,
  MessageSquare,
  Scale,
  ScanLine,
  XCircle,
} from "lucide-react";

import { Badge } from "@/shared/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { TimelineRequestStatus } from "@/features/messages/types";
import type {
  DispositionState,
  HistoryWarning,
  InvalidRequestReason,
  UnsupportedRequestReason,
} from "@/shared/lib/disposition";

const STATE_META: Record<
  DispositionState,
  {
    label: string;
    variant: "success" | "warning" | "destructive" | "outline";
    Icon: typeof CheckCircle2;
  }
> = {
  completed: {
    label: "Completed",
    variant: "success",
    Icon: CheckCircle2,
  },
  refused: {
    label: "Refused",
    variant: "warning",
    Icon: XCircle,
  },
  // Deliberately neutral, not a success variant: the agent answered, but
  // nothing asserted the work was done. Styling it green would restore the
  // exact overclaim `responded` exists to remove.
  responded: {
    label: "Responded",
    variant: "outline",
    Icon: MessageSquare,
  },
  errored: {
    label: "Errored",
    variant: "destructive",
    Icon: AlertTriangle,
  },
};

/**
 * Warnings are diagnostic and do NOT change the outcome. An earlier version
 * forced every one of them into the destructive variant and appended
 * "· disputed", so a duplicate delivery looked identical to a genuine
 * contradiction — which defeated the point of categorizing them at all.
 */
const WARNING_TEXT: Record<HistoryWarning, string> = {
  duplicate_terminal:
    "This request was finalized more than once with the same outcome. Redundant, not contradictory — the outcome stands.",
  ordered_after_terminal:
    "A later disposition sorts after this request was settled. It does not reopen the outcome; the settled result stands.",
};

const INVALID_TEXT: Record<InvalidRequestReason, string> = {
  missing_agent_target:
    "This message is marked as a request but names no agent, so nothing was asked of anyone.",
  malformed_agent_target:
    "This request's agent target is not a valid public key.",
  missing_channel: "This request carries no channel, so it has no scope.",
  multiple_channels:
    "This request names more than one channel, so its scope is ambiguous.",
  target_not_mentioned:
    "This request names an agent but never mentions it, so the request was never actually delivered to it.",
  duplicate_agent_target:
    "This request repeats the same agent target, which is not a well-formed request.",
  unsupported_kind:
    "This message kind cannot carry a request marker in this version.",
};

const UNSUPPORTED_TEXT: Record<UnsupportedRequestReason, string> = {
  multiple_agent_targets:
    "This request addresses more than one agent. This version cannot say whether either agent may answer or both must, so it does not track an outcome for it.",
};

/**
 * NIP-AD status badge for a marked REQUEST message (not the agent's reply —
 * matches the protocol's `e`-tag data model, and is robust when a reply
 * doesn't thread back to a specific request).
 *
 * Reads `TimelineMessage.requestStatus`, which comes straight from the shared
 * verifier — including the target-agent binding that keeps a merely
 * `p`-mentioned human from closing an agent's obligation. No fetch, no local
 * state, and no verification logic of its own. See docs/nips/NIP-AD.md.
 *
 * Invalid and unsupported requests render their own badge rather than nothing.
 * Rendering nothing made them invisible to the reader while the summary chip
 * still counted them, so the one surface that could have explained the
 * problem stayed silent about it.
 */
export function DispositionBadge({
  status,
}: {
  status: TimelineRequestStatus | undefined;
}) {
  if (!status) {
    return null;
  }

  if (status.kind === "invalid" || status.kind === "unsupported") {
    const isInvalid = status.kind === "invalid";
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <Badge
            variant="outline"
            className="mt-1 inline-flex cursor-default items-center gap-1 normal-case tracking-normal"
          >
            {isInvalid ? (
              <CircleSlash className="size-3" aria-hidden="true" />
            ) : (
              <ScanLine className="size-3" aria-hidden="true" />
            )}
            {isInvalid ? "Invalid request" : "Not tracked"}
          </Badge>
        </TooltipTrigger>
        <TooltipContent className="max-w-xs text-left">
          <p>
            {isInvalid
              ? INVALID_TEXT[status.reason]
              : UNSUPPORTED_TEXT[status.reason]}
          </p>
          <p className="mt-1 text-primary-foreground/70">
            {isInvalid
              ? "This is a problem with how the request was sent, not with any agent."
              : "No agent is accountable for this request in this version."}
          </p>
        </TooltipContent>
      </Tooltip>
    );
  }

  // A valid obligation with nothing bound is a real gap, but the badge is
  // about what happened — an absence is shown by the summary, not by a badge
  // on every unanswered request.
  if (status.outcome.kind === "unanswered") {
    return null;
  }

  // A dispute gets its own presentation and never borrows a state label.
  //
  // An earlier version fell back to `latestObservation` for the icon and
  // text, so `completed → refused → responded` rendered as
  // "Responded · disputed" — naming the one state that is NOT part of the
  // contradiction, and taking its reason from a stray later event rather than
  // from either terminal claim. The dispute is between `completed` and
  // `refused`; saying anything else misdescribes it.
  if (status.outcome.kind === "disputed") {
    return (
      <Tooltip>
        <TooltipTrigger asChild>
          <Badge
            variant="destructive"
            className="mt-1 inline-flex cursor-default items-center gap-1 normal-case tracking-normal"
          >
            <Scale className="size-3" aria-hidden="true" />
            Disputed
          </Badge>
        </TooltipTrigger>
        <TooltipContent className="max-w-xs text-left">
          <p>
            This request carries both a <strong>completed</strong> and a{" "}
            <strong>refused</strong> disposition — two contradictory outcomes.
            There is no settled result.
          </p>
          {status.latestObservation ? (
            <p className="mt-1 text-primary-foreground/70">
              Most recent record: {STATE_META[status.latestObservation].label}.
              Shown for context only — it does not settle the request.
            </p>
          ) : null}
          {status.warnings.map((warning) => (
            <p key={warning} className="mt-1 text-primary-foreground/70">
              {WARNING_TEXT[warning]}
            </p>
          ))}
        </TooltipContent>
      </Tooltip>
    );
  }

  const { label, variant, Icon } = STATE_META[status.outcome.state];
  const reason = status.reason.trim();

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Badge
          // Warnings are diagnostic and leave the outcome — and its styling —
          // intact. Only a genuine contradiction changes the presentation,
          // and that is handled above.
          variant={variant}
          className="mt-1 inline-flex cursor-default items-center gap-1 normal-case tracking-normal"
        >
          <Icon className="size-3" aria-hidden="true" />
          {label}
        </Badge>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs text-left">
        {reason ? (
          <p>{reason}</p>
        ) : (
          <p className="text-primary-foreground/70">No reason given.</p>
        )}
        {status.outcome.state === "responded" ? (
          <p className="mt-1 text-primary-foreground/70">
            The agent responded. Nothing has asserted the request is complete.
          </p>
        ) : null}
        {status.warnings.map((warning) => (
          <p key={warning} className="mt-1 text-primary-foreground/70">
            {WARNING_TEXT[warning]}
          </p>
        ))}
      </TooltipContent>
    </Tooltip>
  );
}

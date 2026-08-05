import {
  AlertTriangle,
  CheckCircle2,
  CircleHelp,
  Eye,
  GitBranch,
  Lightbulb,
  LoaderCircle,
  ShieldCheck,
  XCircle,
} from "lucide-react";
import * as React from "react";

import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";

import type { ObserverEvent } from "./agentSessionTypes";
import type { CausalFinding } from "./trustworthySessionTimeline";
import { explainSession } from "./trustworthySessionTimeline";

export function CausalSessionTimeline({
  events,
  findings,
}: {
  events: readonly ObserverEvent[];
  findings: readonly CausalFinding[];
}) {
  const explanation = React.useMemo(
    () => explainSession(events, findings),
    [events, findings],
  );
  if (!explanation) return null;

  const OutcomeIcon =
    explanation.outcome === "failed"
      ? XCircle
      : explanation.outcome === "succeeded"
        ? CheckCircle2
        : explanation.outcome === "in_progress"
          ? LoaderCircle
          : CircleHelp;

  return (
    <section
      aria-label="Session explanation"
      className="mb-3 overflow-hidden rounded-xl border border-border/70 bg-background"
      data-testid="session-explanation"
    >
      <div className="border-b border-border/60 p-4">
        <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          Task
        </p>
        <h3 className="mt-1 text-sm font-semibold">{explanation.task}</h3>
      </div>

      <div className="grid gap-0 sm:grid-cols-[8rem_1fr]">
        <div className="border-b border-border/60 bg-muted/20 p-4 sm:border-r">
          <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
            Outcome
          </p>
          <div className="mt-2 flex items-center gap-2">
            <OutcomeIcon
              aria-hidden
              className={cn(
                "h-5 w-5",
                explanation.outcome === "failed"
                  ? "text-destructive"
                  : explanation.outcome === "succeeded"
                    ? "text-emerald-600"
                    : "text-muted-foreground",
              )}
            />
            <span className="text-sm font-semibold capitalize">
              {explanation.outcome.replace("_", " ")}
            </span>
          </div>
        </div>
        <div className="border-b border-border/60 p-4">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Why
            </p>
            <Badge variant="outline" className="capitalize">
              {explanation.confidence} confidence
            </Badge>
          </div>
          <p className="mt-2 text-sm leading-6">{explanation.why}</p>
        </div>
      </div>

      <div className="border-b border-border/60 p-4">
        <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          Evidence
        </p>
        <ol className="mt-2 space-y-2">
          {explanation.evidence.map((event) => {
            const Icon =
              event.class === "decision"
                ? ShieldCheck
                : event.class === "inference"
                  ? GitBranch
                  : Eye;
            return (
              <li
                className="flex items-start gap-2 text-xs"
                data-causal-event-class={event.class}
                key={event.id}
              >
                <Icon
                  aria-hidden
                  className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground"
                />
                <div className="min-w-0 flex-1">
                  <span className="font-medium">{event.title}</span>
                  <span className="text-muted-foreground">
                    {" "}
                    · {event.sourceSystem}
                  </span>
                </div>
              </li>
            );
          })}
        </ol>
      </div>

      <div className="border-b border-border/60 p-4">
        <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          Unknowns
        </p>
        {explanation.unknowns.length > 0 ? (
          <ul className="mt-2 space-y-2">
            {explanation.unknowns.map((gap) => (
              <li
                className="flex items-start gap-2 text-xs text-amber-700 dark:text-amber-300"
                key={gap.id}
              >
                <AlertTriangle
                  aria-hidden
                  className="mt-0.5 h-3.5 w-3.5 shrink-0"
                />
                <span>{gap.title}</span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="mt-2 text-xs text-muted-foreground">
            No known evidence gaps.
          </p>
        )}
      </div>

      <div className="bg-primary/[0.04] p-4">
        <div className="flex items-start gap-2">
          <Lightbulb
            aria-hidden
            className="mt-0.5 h-4 w-4 shrink-0 text-primary"
          />
          <div>
            <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Suggested next action
            </p>
            <p className="mt-1 text-sm font-medium">{explanation.nextAction}</p>
          </div>
        </div>
      </div>

      <p className="border-t border-border/60 px-4 py-3 text-xs text-muted-foreground">
        Activity and raw telemetry continue below as supporting detail.
      </p>
    </section>
  );
}

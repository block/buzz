import { AlertTriangle, Eye, GitBranch, ShieldCheck } from "lucide-react";
import * as React from "react";

import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";

import type { ObserverEvent } from "./agentSessionTypes";
import type { CausalFinding } from "./trustworthySessionTimeline";
import { buildTrustworthySessionTimeline } from "./trustworthySessionTimeline";

export function CausalSessionTimeline({
  events,
  findings,
}: {
  events: readonly ObserverEvent[];
  findings: readonly CausalFinding[];
}) {
  const timeline = React.useMemo(
    () => buildTrustworthySessionTimeline(events, findings),
    [events, findings],
  );
  if (timeline.length === 0) return null;

  const gaps = timeline.filter((event) => event.class === "gap");
  const visible = timeline.slice(-12);

  return (
    <section
      aria-label="Session causal timeline"
      className="mb-3 rounded-lg border border-border/70 bg-muted/20 p-3"
      data-testid="trustworthy-session-timeline"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h3 className="text-sm font-semibold">
            Why did this agent behave this way?
          </h3>
          <p className="text-xs text-muted-foreground">
            Evidence and known visibility gaps, ordered across observing
            systems.
          </p>
        </div>
        <Badge variant={gaps.length > 0 ? "outline" : "secondary"}>
          {gaps.length > 0
            ? `${gaps.length} coverage gap${gaps.length === 1 ? "" : "s"}`
            : "Complete coverage"}
        </Badge>
      </div>
      <ol className="mt-3 space-y-2">
        {visible.map((event) => {
          const Icon =
            event.class === "gap"
              ? AlertTriangle
              : event.class === "decision"
                ? ShieldCheck
                : event.class === "inference"
                  ? GitBranch
                  : Eye;
          return (
            <li
              className={cn(
                "rounded-md border px-2.5 py-2 text-xs",
                event.class === "gap"
                  ? "border-amber-500/40 bg-amber-500/10"
                  : "border-border/60 bg-background/60",
              )}
              data-causal-event-class={event.class}
              key={event.id}
            >
              <div className="flex items-start gap-2">
                <Icon aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="font-medium">{event.title}</span>
                    <Badge variant="outline">{event.sourceSystem}</Badge>
                    <span className="text-muted-foreground">
                      {event.observerRole}
                    </span>
                  </div>
                  <p className="mt-1 text-muted-foreground">{event.detail}</p>
                </div>
              </div>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

import {
  AlertTriangle,
  BriefcaseBusiness,
  CircleDot,
  ExternalLink,
  ShieldAlert,
} from "lucide-react";

import { managedAgentRuntimePresentation } from "@/features/agents/managedAgentRuntimeStatus";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";

export function ManagedAgentRuntimeSummary({
  className,
  runtime,
}: {
  className?: string;
  runtime: ManagedAgentRuntimeStatus | undefined;
}) {
  if (!runtime) return null;
  const presentation = managedAgentRuntimePresentation(runtime);
  const assignment = runtime.activeAssignment;
  const job = runtime.activeJob;
  const sourceChannelId = assignment?.channelId ?? job?.channelId;
  const sourceEventId = assignment?.sourceEventId ?? job?.sourceEventId;
  const sourceHref =
    sourceChannelId && sourceEventId
      ? `buzz://message?channel=${encodeURIComponent(sourceChannelId)}&id=${encodeURIComponent(sourceEventId)}`
      : null;

  return (
    <section
      aria-label="Persistent runtime status"
      className={cn("min-w-0 space-y-2", className)}
      data-runtime-lifecycle={runtime.lifecycle}
      data-runtime-pid={runtime.pid ?? undefined}
      data-runtime-pubkey={runtime.pubkey}
      data-runtime-relay-url={runtime.relayUrl}
      data-runtime-progress-seq={job?.progressSeq}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Badge className="gap-1.5" variant={presentation.variant}>
          <CircleDot aria-hidden className="h-3.5 w-3.5" />
          {presentation.label}
        </Badge>
        {job ? (
          <Badge
            className="max-w-full gap-1.5 normal-case tracking-normal"
            variant="outline"
          >
            <BriefcaseBusiness aria-hidden className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">
              {job.state.replaceAll("_", " ")} · job {job.jobId}
            </span>
          </Badge>
        ) : null}
      </div>

      {assignment ? (
        <div className="min-w-0 text-xs">
          <p className="truncate font-medium text-foreground">
            {assignment.summary}
          </p>
          <p className="mt-0.5 text-muted-foreground">
            {assignment.state.replaceAll("_", " ")}
            {assignment.activeJobId ? ` · job ${assignment.activeJobId}` : ""}
          </p>
          {assignment.hasBlocker || assignment.state === "blocked" ? (
            <p
              className="mt-1 flex items-start gap-1.5 text-destructive"
              role="status"
            >
              <AlertTriangle
                aria-hidden
                className="mt-0.5 h-3.5 w-3.5 shrink-0"
              />
              <span>Blocked — see the source thread for blocker details.</span>
            </p>
          ) : assignment.state === "needs_approval" ? (
            <p
              className="mt-1 flex items-start gap-1.5 text-amber-600 dark:text-amber-400"
              role="status"
            >
              <ShieldAlert
                aria-hidden
                className="mt-0.5 h-3.5 w-3.5 shrink-0"
              />
              <span>Approval required before work can continue.</span>
            </p>
          ) : null}
        </div>
      ) : presentation.detail ? (
        <p className="text-xs text-muted-foreground">{presentation.detail}</p>
      ) : null}

      {job ? (
        <div className="min-w-0 rounded-md border border-border/60 bg-muted/20 px-2.5 py-2 text-xs">
          <p className="truncate font-medium text-foreground">{job.summary}</p>
          <p className="mt-0.5 text-muted-foreground">
            Progress update {job.progressSeq} · attempt {job.attempt} · relay{" "}
            {job.publicationState.replaceAll("_", " ")}
          </p>
        </div>
      ) : null}

      {sourceHref ? (
        <a
          className="inline-flex items-center gap-1 text-xs font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          href={sourceHref}
        >
          <ExternalLink aria-hidden className="h-3.5 w-3.5" />
          Source thread
        </a>
      ) : null}

      {job?.publicationState === "failed" ? (
        <p
          className="flex items-start gap-1.5 text-xs text-destructive"
          role="status"
        >
          <AlertTriangle aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>
            Job state is saved locally, but its latest relay publication failed.
          </span>
        </p>
      ) : null}
    </section>
  );
}

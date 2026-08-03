import {
  AlertTriangle,
  CheckCircle2,
  CircleDot,
  ExternalLink,
  FileText,
  OctagonX,
  Timer,
  XCircle,
} from "lucide-react";

import type {
  AgentJobState,
  AgentJobView,
} from "@/features/messages/lib/agentJobProjection";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { useNow } from "@/shared/lib/useNow";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";

export type AgentJobCardProps = {
  job: AgentJobView;
  nowMs?: number;
  onCancel?: (job: AgentJobView) => void;
};

const TERMINAL_STATE: Record<AgentJobState, boolean> = {
  requested: false,
  accepted: false,
  running: false,
  cancelling: false,
  succeeded: true,
  failed: true,
  cancelled: true,
  lost: true,
};
const JOB_STATE_PRESENTATION: Record<
  AgentJobState,
  {
    label: string;
    variant: "default" | "secondary" | "warning" | "success" | "destructive";
    Icon: typeof CircleDot;
  }
> = {
  requested: { label: "Requested", variant: "secondary", Icon: Timer },
  accepted: { label: "Accepted", variant: "default", Icon: CircleDot },
  running: { label: "Running", variant: "default", Icon: CircleDot },
  cancelling: { label: "Cancelling", variant: "warning", Icon: Timer },
  succeeded: { label: "Succeeded", variant: "success", Icon: CheckCircle2 },
  failed: { label: "Failed", variant: "destructive", Icon: XCircle },
  cancelled: { label: "Cancelled", variant: "secondary", Icon: OctagonX },
  lost: { label: "Lost", variant: "destructive", Icon: AlertTriangle },
};

function LiveJobElapsed({ startedAt }: { startedAt: number }) {
  const now = useNow(1_000);
  return <>{formatElapsed(Math.max(0, now - startedAt * 1_000))}</>;
}

export function AgentJobCard({ job, nowMs, onCancel }: AgentJobCardProps) {
  const presentation = JOB_STATE_PRESENTATION[job.state];
  const StatusIcon = presentation.Icon;
  const elapsedEndMs =
    job.finishedAt != null ? job.finishedAt * 1_000 : (nowMs ?? null);
  const staticElapsed =
    job.startedAt != null && elapsedEndMs != null
      ? formatElapsed(Math.max(0, elapsedEndMs - job.startedAt * 1_000))
      : null;
  const sourceHref = job.sourceEventId
    ? `buzz://message?channel=${encodeURIComponent(job.channelId)}&id=${encodeURIComponent(job.sourceEventId)}`
    : null;
  const isTerminal = TERMINAL_STATE[job.state];
  const cancelDisabled = isTerminal || onCancel == null;

  return (
    <section
      aria-label={`Agent job ${job.jobId}`}
      className="w-full max-w-2xl rounded-lg border border-border/70 bg-muted/20 px-3 py-3 shadow-xs sm:px-4"
      data-job-state={job.state}
      data-progress-seq={job.progressSeq ?? undefined}
      data-testid={`agent-job-${job.jobId}`}
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Badge className="gap-1.5" variant={presentation.variant}>
              <StatusIcon aria-hidden className="h-3.5 w-3.5" />
              {presentation.label}
            </Badge>
            {job.startedAt != null ? (
              <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                <Timer aria-hidden className="h-3.5 w-3.5" />
                <span>
                  {staticElapsed ?? (
                    <LiveJobElapsed startedAt={job.startedAt} />
                  )}
                </span>
              </span>
            ) : null}
            {job.attempt != null ? (
              <span className="text-xs text-muted-foreground">
                Attempt {job.attempt}
              </span>
            ) : null}
          </div>
          <p className="mt-2 text-sm font-medium leading-5 text-foreground">
            {job.summary}
          </p>
          <p className="mt-1 truncate font-mono text-xs text-muted-foreground">
            Job {job.jobId}
          </p>
        </div>

        <Button
          aria-label={`Cancel job ${job.jobId}`}
          disabled={cancelDisabled}
          onClick={() => onCancel?.(job)}
          size="xs"
          title={
            isTerminal
              ? "This job is already finished"
              : onCancel
                ? "Request cancellation"
                : "Cancellation is unavailable here"
          }
          type="button"
          variant="outline"
        >
          Cancel
        </Button>
      </div>

      {sourceHref || job.artifacts.length > 0 ? (
        <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-border/60 pt-3 text-xs">
          {sourceHref ? (
            <a
              className="inline-flex items-center gap-1 font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
              href={sourceHref}
            >
              <ExternalLink aria-hidden className="h-3.5 w-3.5" />
              Source message
            </a>
          ) : null}
          {job.artifacts.map((artifact) => (
            <a
              className="inline-flex max-w-full items-center gap-1 font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
              href={artifact.uri}
              key={`${artifact.name}:${artifact.uri}`}
              rel="noreferrer"
              target="_blank"
              title={artifact.sha256 ? `SHA-256 ${artifact.sha256}` : undefined}
            >
              <FileText aria-hidden className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{artifact.name}</span>
            </a>
          ))}
        </div>
      ) : null}

      {job.publicationFailed ? (
        <p
          className="mt-3 flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
          role="status"
        >
          <AlertTriangle aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          Result saved locally, but its relay publication failed.
        </p>
      ) : null}

      {isTerminal ? (
        <div className="mt-3 border-t border-border/60 pt-3 text-xs text-muted-foreground">
          {job.state === "succeeded" ? (
            <p>
              Completed{job.exitCode != null ? ` · exit ${job.exitCode}` : ""}
            </p>
          ) : (
            <p role="status">
              {presentation.label}
              {job.errorCode ? ` · ${job.errorCode}` : ""}
            </p>
          )}
        </div>
      ) : null}
    </section>
  );
}

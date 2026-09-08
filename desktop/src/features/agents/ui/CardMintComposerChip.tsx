import { Loader2, Sparkles, X } from "lucide-react";

import {
  dismissCardMintJob,
  useCardMintJobs,
  viewMintedCardJob,
} from "@/features/agents/cardMintStore";
import { Shimmer } from "@/shared/ui/Shimmer";

/**
 * Composer-rail status for background card mints: a live "Minting card…"
 * row while the ~2–3 minute mint runs, then a clickable "card is ready" row
 * (opens the viewer) or a dismissible failure row. Rendered inside the
 * composer dock's bottom activity rail next to agent-working/typing rows.
 */
export function CardMintComposerChip() {
  const jobs = useCardMintJobs();
  if (jobs.length === 0) return null;

  return (
    <div
      className="flex min-w-0 flex-col justify-center gap-0.5"
      data-testid="card-mint-composer-chip"
    >
      {jobs.map((job) => {
        if (job.phase === "minting") {
          return (
            <span
              className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground"
              data-testid="card-mint-status-minting"
              key={job.jobId}
            >
              <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin opacity-70" />
              <Shimmer className="-my-px truncate py-px">
                {`Minting ${job.input.agentName}’s card… (takes a few minutes)`}
              </Shimmer>
            </span>
          );
        }
        if (job.phase === "done") {
          return (
            <button
              className="flex min-w-0 items-center gap-1.5 text-xs text-primary hover:underline"
              data-testid="card-mint-status-done"
              key={job.jobId}
              onClick={() => viewMintedCardJob(job.jobId)}
              type="button"
            >
              <Sparkles className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">
                {`${job.input.agentName}’s card is ready — view it`}
              </span>
            </button>
          );
        }
        return (
          <span
            className="flex min-w-0 items-center gap-1.5 text-xs text-destructive"
            data-testid="card-mint-status-error"
            key={job.jobId}
          >
            <span className="truncate">
              {`Minting ${job.input.agentName}’s card failed`}
            </span>
            <button
              aria-label="Dismiss failed mint"
              className="shrink-0 opacity-70 hover:opacity-100"
              onClick={() => dismissCardMintJob(job.jobId)}
              type="button"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </span>
        );
      })}
    </div>
  );
}

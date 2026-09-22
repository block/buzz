import { Loader2, Sparkles, X } from "lucide-react";

import { useTranslation } from "@/i18n";
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
  const { t } = useTranslation();
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
                {t("agents.card-mint.chip-minting", {
                  name: job.input.agentName,
                })}
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
                {t("agents.card-mint.chip-ready", {
                  name: job.input.agentName,
                })}
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
              {t("agents.card-mint.chip-failed", {
                name: job.input.agentName,
              })}
            </span>
            <button
              aria-label={t("agents.card-mint.dismiss")}
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

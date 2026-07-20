import { Loader2 } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { useActiveUploadJobs } from "@/features/messages/lib/uploadJobsStore";

type UploadIndicatorProps = {
  /** Jump to the channel an upload belongs to. */
  onOpenChannel: (channelId: string) => void;
};

/**
 * Channel-independent, always-mounted upload indicator.
 *
 * Uploads run in the workspace-scoped `uploadJobsStore`, not in any composer,
 * so this pill stays put while you navigate around. It shows aggregate
 * progress across every in-flight upload and, on click, opens the channel of
 * the oldest active one.
 */
export function UploadIndicator({ onOpenChannel }: UploadIndicatorProps) {
  const jobs = useActiveUploadJobs();
  if (jobs.length === 0) return null;

  const pct = Math.round(
    jobs.reduce((sum, job) => sum + job.pct, 0) / jobs.length,
  );
  const label =
    jobs.length === 1
      ? (jobs[0].filename ?? "Uploading…")
      : `Uploading ${jobs.length} files`;

  return (
    <button
      type="button"
      onClick={() => onOpenChannel(jobs[0].channelId)}
      data-testid="global-upload-indicator"
      className={cn(
        "fixed bottom-4 right-4 z-50 flex items-center gap-2 rounded-full",
        "border border-border/60 bg-background/95 px-3 py-1.5 shadow-lg backdrop-blur",
        "text-sm text-foreground/90 transition-colors hover:bg-accent",
      )}
      title={`${label} — ${pct}%`}
    >
      <Loader2 className="size-4 animate-spin text-foreground/70" />
      <span className="max-w-40 truncate">{label}</span>
      <span className="tabular-nums text-foreground/60">{pct}%</span>
    </button>
  );
}

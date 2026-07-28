import {
  AlertTriangle,
  CalendarPlus,
  CheckCircle2,
  ShieldAlert,
} from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { PlanningFinding } from "../domain/deterministicChecks";

function severityIcon(severity: PlanningFinding["severity"]) {
  if (severity === "critical")
    return <ShieldAlert className="h-4 w-4 text-destructive" />;
  return <AlertTriangle className="h-4 w-4 text-amber-500" />;
}

export function PlanningReviewPanel({
  findings,
  onOpenChange,
  onReviewEvent,
  open,
}: {
  findings: readonly PlanningFinding[];
  onOpenChange: (open: boolean) => void;
  onReviewEvent: (finding: PlanningFinding) => void;
  open: boolean;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Planning Review</DialogTitle>
        </DialogHeader>
        <div className="flex items-center justify-between gap-3 rounded border bg-muted/30 p-3">
          <div>
            <p className="text-sm font-medium">Planning assurance</p>
            <p className="text-xs text-muted-foreground">
              Checks the approved calendar for missing prerequisites and
              conflicting source dates.
            </p>
          </div>
          <span className="rounded-full border px-2 py-1 text-2xs uppercase tracking-wide text-muted-foreground">
            Rules only
          </span>
        </div>
        {findings.length ? (
          <div className="grid max-h-[32rem] gap-3 overflow-y-auto">
            {findings.map((finding) => (
              <article className="rounded border p-4" key={finding.id}>
                <div className="flex items-start gap-2">
                  {severityIcon(finding.severity)}
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <strong className="text-sm">{finding.title}</strong>
                      <span className="text-2xs uppercase tracking-wide text-muted-foreground">
                        {finding.category.replace(/([A-Z])/g, " $1")} ·{" "}
                        {Math.round(finding.confidence * 100)}% confidence
                      </span>
                    </div>
                    <p className="mt-2 text-sm text-muted-foreground">
                      {finding.rationale}
                    </p>
                    {finding.proposedEvent ? (
                      <button
                        className="mt-3 rounded bg-primary px-3 py-2 text-sm text-primary-foreground"
                        onClick={() => onReviewEvent(finding)}
                        type="button"
                      >
                        <CalendarPlus className="mr-1 inline h-4 w-4" />
                        Review proposed event
                      </button>
                    ) : (
                      <p className="mt-3 text-xs text-muted-foreground">
                        Review the affected source entries before changing the
                        approved calendar.
                      </p>
                    )}
                  </div>
                </div>
              </article>
            ))}
          </div>
        ) : (
          <div className="flex items-center gap-3 rounded border border-emerald-500/30 bg-emerald-500/10 p-4">
            <CheckCircle2 className="h-5 w-5 text-emerald-600" />
            <div>
              <p className="text-sm font-medium">No planning gaps found</p>
              <p className="text-xs text-muted-foreground">
                The current rule set found no missing sailing prerequisite or
                conflicting source date.
              </p>
            </div>
          </div>
        )}
        <p className="text-2xs text-muted-foreground">
          Findings are advisory. Proposed events always open for review and are
          not added until you save them.
        </p>
      </DialogContent>
    </Dialog>
  );
}

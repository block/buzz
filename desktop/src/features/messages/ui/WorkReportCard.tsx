import { CheckCircle2, CircleAlert, FileCheck2 } from "lucide-react";

import type {
  WorkReport,
  WorkReportStatus,
} from "@/features/messages/lib/workReport";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";

const STATUS_PRESENTATION: Record<
  WorkReportStatus,
  { label: string; variant: "success" | "info" | "warning" | "destructive" }
> = {
  completed: { label: "Completed", variant: "success" },
  in_review: { label: "In review", variant: "info" },
  needs_decision: { label: "Decision needed", variant: "warning" },
  blocked: { label: "Blocked", variant: "warning" },
  failed: { label: "Failed", variant: "destructive" },
};

function ReportList({ label, values }: { label: string; values: string[] }) {
  if (values.length === 0) return null;
  return (
    <section className="space-y-1.5">
      <h4 className="text-xs font-semibold text-muted-foreground">{label}</h4>
      <ul className="space-y-1 text-sm text-foreground/90">
        {values.map((value) => (
          <li className="flex gap-2" key={value}>
            <span aria-hidden className="text-muted-foreground">
              •
            </span>
            <span className="min-w-0 break-words">{value}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}

export function WorkReportCard({
  report,
  conversationVisible,
  onToggleConversation,
}: {
  report: WorkReport;
  conversationVisible: boolean;
  onToggleConversation: () => void;
}) {
  const status = STATUS_PRESENTATION[report.status];
  const Icon = report.status === "completed" ? CheckCircle2 : CircleAlert;
  return (
    <article
      className="rounded-2xl border border-border/70 bg-card/95 p-4 shadow-xs"
      data-testid="work-report-card"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <FileCheck2 className="size-4 shrink-0 text-muted-foreground" />
          <h3 className="text-sm font-semibold">Work result</h3>
        </div>
        <Badge variant={status.variant}>
          <Icon className="mr-1 size-3" />
          {status.label}
        </Badge>
      </div>
      <p className="mt-3 text-sm leading-6 text-foreground">{report.outcome}</p>
      <div className="mt-4 space-y-3">
        <ReportList label="Deliverables" values={report.deliverables} />
        <ReportList label="Decisions" values={report.decisions} />
        <ReportList label="Verification" values={report.verification} />
        <ReportList label="Risks" values={report.risks} />
        <ReportList label="Next actions" values={report.nextActions} />
      </div>
      <div className="mt-4 border-t border-border/60 pt-3">
        <Button
          aria-expanded={conversationVisible}
          data-testid="work-report-conversation-toggle"
          onClick={onToggleConversation}
          size="xs"
          variant="ghost"
        >
          {conversationVisible ? "Hide conversation" : "Show conversation"}
        </Button>
      </div>
    </article>
  );
}

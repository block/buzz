import { AlertTriangle, CheckCircle2 } from "lucide-react";

import type { AdviserContribution } from "@/features/command-console/domain/briefContracts";
import { Badge } from "@/shared/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";
import { Progress } from "@/shared/ui/progress";

import { SourceCitationLink } from "./SourceCitationLink";

const ADVISER_LABELS: Record<AdviserContribution["adviser"], string> = {
  operations: "Operations",
  navigation: "Navigation",
  daily_routine: "Daily Routine",
  reporting: "Reporting",
  plans: "Plans",
};

export function AdviserContributionCard({
  contribution,
}: {
  contribution: AdviserContribution;
}) {
  const confidence = Math.round(contribution.confidence * 100);
  return (
    <Card data-testid={`adviser-${contribution.adviser}`}>
      <CardHeader className="gap-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <CardTitle className="text-base">
            {ADVISER_LABELS[contribution.adviser]}
          </CardTitle>
          <Badge variant="info">{confidence}% confidence</Badge>
        </div>
        <Progress
          aria-label={`${ADVISER_LABELS[contribution.adviser]} confidence`}
          className="motion-reduce:[&>div]:transition-none"
          value={confidence}
        />
      </CardHeader>
      <CardContent className="space-y-4">
        <div>
          <h4 className="text-sm font-semibold">Findings</h4>
          {contribution.findings.length === 0 ? (
            <p className="mt-2 text-sm text-muted-foreground">
              No supported finding was returned.
            </p>
          ) : (
            <ul className="mt-2 space-y-2 text-sm">
              {contribution.findings.map((finding) => (
                <li key={`${finding.text}-${finding.sourceIds.join("-")}`}>
                  <span>{finding.text}</span>{" "}
                  <span>
                    <span className="sr-only">Citations: </span>
                    {finding.sourceIds.map((sourceId) => (
                      <SourceCitationLink key={sourceId} sourceId={sourceId} />
                    ))}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>

        {contribution.limitations.length > 0 ? (
          <div className="rounded-lg border border-warning/30 bg-warning/10 p-3">
            <h4 className="flex items-center gap-2 text-sm font-semibold">
              <AlertTriangle className="h-4 w-4" aria-hidden="true" />
              Limitations
            </h4>
            <ul className="mt-2 list-disc space-y-1 pl-5 text-sm">
              {contribution.limitations.map((limitation) => (
                <li key={limitation}>{limitation}</li>
              ))}
            </ul>
          </div>
        ) : null}

        {contribution.dissent.length > 0 ? (
          <div className="rounded-lg border p-3">
            <h4 className="text-sm font-semibold">Dissent retained</h4>
            <ul className="mt-2 list-disc space-y-1 pl-5 text-sm">
              {contribution.dissent.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </div>
        ) : null}

        {contribution.proposedActions.length > 0 ? (
          <div>
            <h4 className="text-sm font-semibold">Workspace proposals</h4>
            <ul className="mt-2 space-y-2">
              {contribution.proposedActions.map((proposal) => (
                <li
                  className="flex items-start gap-2 rounded-lg border bg-muted/30 p-3 text-sm"
                  key={proposal.actionId}
                >
                  <CheckCircle2
                    className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <span className="min-w-0 flex-1">{proposal.text}</span>
                  <Badge variant="secondary">Pending proposal</Badge>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}

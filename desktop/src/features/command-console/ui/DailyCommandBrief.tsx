import { AlertTriangle, Ban, Play, RefreshCw } from "lucide-react";

import {
  BRIEF_SECTIONS,
  type BriefRunState,
  type BriefRunStatus,
  type BriefSchedule,
  type BriefSection,
  type PublishedCommandBrief,
} from "@/features/command-console/domain/briefContracts";
import type { CommandBriefScheduleUpdate } from "@/shared/api/tauriCommandBrief";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";
import { Progress } from "@/shared/ui/progress";
import { AdviserContributionCard } from "./AdviserContributionCard";
import { BriefScheduleControls } from "./BriefScheduleControls";
import { SourceLedger } from "./SourceLedger";

const SECTION_LABELS: Record<BriefSection, string> = {
  today: "Today at a glance",
  operations: "Operational priorities and risks",
  navigation: "Navigation considerations",
  daily_routine: "Daily routine and calendar",
  reports: "Reports and returns due",
  planning_30_60_90: "30, 60 and 90 day planning horizon",
  decisions: "Decisions required",
  conflicts_and_gaps: "Conflicts and gaps",
  sources: "Sources",
};

const STATE_LABELS: Record<BriefRunState, string> = {
  queued: "Queued",
  collecting_sources: "Collecting sources",
  running_specialists: "Running specialists",
  consolidating: "Consolidating",
  persisting: "Securing brief",
  completed: "Complete",
  degraded: "Complete with limitations",
  cancelled: "Cancelled",
  failed: "Failed",
};

const PROGRESS: Record<BriefRunState, number | null> = {
  queued: 5,
  collecting_sources: 20,
  running_specialists: 50,
  consolidating: 75,
  persisting: 90,
  completed: 100,
  degraded: 100,
  cancelled: 100,
  failed: 100,
};

const ACTIVE_STATES = new Set<BriefRunState>([
  "queued",
  "collecting_sources",
  "running_specialists",
  "consolidating",
  "persisting",
]);

function statusVariant(state: BriefRunState) {
  if (state === "completed") return "success" as const;
  if (state === "degraded") return "warning" as const;
  if (state === "failed") return "destructive" as const;
  return "secondary" as const;
}

function BriefStatus({
  status,
  busy,
  onCancel,
}: {
  status: BriefRunStatus | null;
  busy: boolean;
  onCancel: () => void;
}) {
  if (!status) return null;
  const active = ACTIVE_STATES.has(status.state);
  return (
    <Card aria-live="polite" data-testid="daily-command-brief-status">
      <CardHeader className="gap-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <CardTitle className="text-base">Generation status</CardTitle>
          <Badge variant={statusVariant(status.state)}>
            {STATE_LABELS[status.state]}
          </Badge>
        </div>
        <Progress
          aria-label={`Daily Command Brief: ${STATE_LABELS[status.state]}`}
          className="motion-reduce:[&>div]:transition-none"
          value={PROGRESS[status.state]}
        />
      </CardHeader>
      <CardContent className="flex flex-wrap items-start justify-between gap-3">
        <div className="text-sm text-muted-foreground">
          <p>
            Run <span className="font-mono">{status.runId}</span>
          </p>
          <p>
            Updated <time dateTime={status.updatedAt}>{status.updatedAt}</time>
          </p>
          {status.error ? (
            <p className="mt-2 text-destructive">{status.error}</p>
          ) : null}
        </div>
        {active ? (
          <Button
            disabled={busy}
            onClick={onCancel}
            type="button"
            variant="outline"
          >
            <Ban aria-hidden="true" />
            Cancel generation
          </Button>
        ) : null}
      </CardContent>
    </Card>
  );
}

function BriefSections({ published }: { published: PublishedCommandBrief }) {
  const { brief } = published;
  return (
    <>
      <div className="grid gap-4 lg:grid-cols-2">
        {BRIEF_SECTIONS.map((section) => (
          <Card key={section}>
            <CardHeader className="pb-3">
              <CardTitle className="text-base">
                {SECTION_LABELS[section]}
              </CardTitle>
            </CardHeader>
            <CardContent>
              {brief.sections[section].length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No supported finding was available for this section.
                </p>
              ) : (
                <ul className="space-y-2 text-sm">
                  {brief.sections[section].map((finding) => (
                    <li key={`${finding.text}-${finding.sourceIds.join("-")}`}>
                      {finding.text}{" "}
                      {finding.sourceIds.map((sourceId) => (
                        <a
                          className="font-medium text-primary underline underline-offset-2"
                          href={`#command-brief-source-${sourceId}`}
                          key={sourceId}
                        >
                          [{sourceId}]
                        </a>
                      ))}
                    </li>
                  ))}
                </ul>
              )}
            </CardContent>
          </Card>
        ))}
      </div>
      <SourceLedger
        entries={brief.sourceLedger}
        freshness={brief.sourceFreshness}
      />
    </>
  );
}

export type DailyCommandBriefProps = {
  readonly status: BriefRunStatus | null;
  readonly history: readonly BriefRunStatus[];
  readonly latest: PublishedCommandBrief | null;
  readonly schedule: BriefSchedule | null;
  readonly loading: boolean;
  readonly busy: boolean;
  readonly error: string | null;
  readonly onGenerate: () => void;
  readonly onCancel: () => void;
  readonly onScheduleChange: (update: CommandBriefScheduleUpdate) => void;
};

export function DailyCommandBrief({
  status,
  history,
  latest,
  schedule,
  loading,
  busy,
  error,
  onGenerate,
  onCancel,
  onScheduleChange,
}: DailyCommandBriefProps) {
  return (
    <section
      aria-labelledby="daily-command-brief-heading"
      className="space-y-5"
      data-testid="daily-command-brief"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2
            className="text-xl font-semibold"
            id="daily-command-brief-heading"
          >
            Daily Command Brief
          </h2>
          <p className="text-sm text-muted-foreground">
            Local-model advice grounded in the frozen OFFICIAL knowledge
            snapshot.
          </p>
        </div>
        <Button
          disabled={busy || ACTIVE_STATES.has(status?.state ?? "completed")}
          onClick={onGenerate}
          type="button"
        >
          {loading ? (
            <RefreshCw
              className="motion-safe:animate-spin"
              aria-hidden="true"
            />
          ) : (
            <Play aria-hidden="true" />
          )}
          Generate Daily Brief
        </Button>
      </div>

      {error ? (
        <Alert variant="destructive">
          <AlertTitle>Daily Command Brief unavailable</AlertTitle>
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      ) : null}

      <BriefStatus status={status} busy={busy} onCancel={onCancel} />

      {history.length > 0 ? (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base">Lifecycle history</CardTitle>
          </CardHeader>
          <CardContent>
            <ol className="grid gap-2 sm:grid-cols-2">
              {history.map((entry) => (
                <li
                  className="flex items-center justify-between gap-3 rounded-lg border p-3 text-sm"
                  key={`${entry.runId}-${entry.updatedAt}-${entry.state}`}
                >
                  <span>{STATE_LABELS[entry.state]}</span>
                  <time
                    className="text-muted-foreground"
                    dateTime={entry.updatedAt}
                  >
                    {entry.updatedAt}
                  </time>
                </li>
              ))}
            </ol>
          </CardContent>
        </Card>
      ) : null}

      {!latest ? (
        <Card>
          <CardContent className="py-8 text-center">
            <p className="font-medium">
              No Daily Command Brief has been generated.
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              Generation remains unavailable until the native identity, local
              model, and protected knowledge sources pass readiness checks.
            </p>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-5">
          <Card>
            <CardContent className="flex flex-wrap items-start justify-between gap-4 py-4">
              <div>
                <p className="font-medium">
                  Generated{" "}
                  <time dateTime={latest.brief.generatedAt}>
                    {latest.brief.generatedAt}
                  </time>
                </p>
                <p className="text-sm text-muted-foreground">
                  Snapshot{" "}
                  <span className="break-all font-mono">
                    {latest.brief.snapshotId}
                  </span>
                </p>
              </div>
              <Badge
                variant={
                  latest.publicationState === "published"
                    ? "success"
                    : "warning"
                }
              >
                {latest.publicationState === "published"
                  ? "Signed and published"
                  : "Relay publication queued — offline capable"}
              </Badge>
            </CardContent>
          </Card>

          <Alert>
            <AlertTitle className="flex items-center gap-2">
              <AlertTriangle className="h-4 w-4" aria-hidden="true" />
              Advisory, non-accredited decision support
            </AlertTitle>
            <AlertDescription>
              {latest.brief.advisoryLimitation}
            </AlertDescription>
          </Alert>

          {latest.brief.missingInformation.length > 0 ? (
            <Alert>
              <AlertTitle>Missing information</AlertTitle>
              <AlertDescription>
                <ul className="list-disc pl-5">
                  {latest.brief.missingInformation.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </AlertDescription>
            </Alert>
          ) : null}

          <BriefSections published={latest} />

          <section aria-labelledby="adviser-contributions-heading">
            <h3
              className="mb-3 text-base font-semibold"
              id="adviser-contributions-heading"
            >
              Specialist adviser contributions
            </h3>
            <div className="grid gap-4 lg:grid-cols-2">
              {latest.brief.contributions.map((contribution) => (
                <AdviserContributionCard
                  contribution={contribution}
                  key={contribution.adviser}
                />
              ))}
            </div>
          </section>
        </div>
      )}

      {schedule ? (
        <BriefScheduleControls
          disabled={busy}
          onChange={onScheduleChange}
          schedule={schedule}
        />
      ) : null}
    </section>
  );
}

import { AlertTriangle, Ban, Play, RefreshCw } from "lucide-react";

import type {
  BriefRunState,
  BriefRunStatus,
  BriefSchedule,
  PublishedCommandBrief,
} from "@/features/command-console/domain/briefContracts";
import type { CommandConsoleStatusViewModel } from "@/features/command-console/hooks/useCommandConsoleStatus";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/shared/ui/card";
import { Progress } from "@/shared/ui/progress";

import type { CommandBriefSchedulePatch } from "../hooks/useDailyCommandBrief";
import { STATE_LABELS, SECTION_LABELS } from "./briefPresentation";
import { BriefEvidenceDisclosure } from "./BriefEvidenceDisclosure";
import { BriefScheduleControls } from "./BriefScheduleControls";
import { BriefSectionCard } from "./BriefSectionCard";
import { SourceCitationLink } from "./SourceCitationLink";

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

function WatchItems({ published }: { published: PublishedCommandBrief }) {
  const { brief } = published;
  const conflicts = brief.sections.conflicts_and_gaps;
  const hasItems =
    brief.degradedSections.length > 0 ||
    brief.missingInformation.length > 0 ||
    brief.dissent.length > 0 ||
    conflicts.length > 0;

  if (!hasItems) return null;

  return (
    <Alert className="border border-warning/30 bg-warning/10">
      <AlertTitle className="flex items-center gap-2">
        <AlertTriangle aria-hidden="true" className="h-4 w-4" />
        Watch items
      </AlertTitle>
      <AlertDescription className="grid gap-4 md:grid-cols-2">
        {brief.degradedSections.length > 0 ? (
          <div>
            <h4 className="font-semibold">Complete with limitations</h4>
            <ul className="mt-1 list-disc pl-5">
              {brief.degradedSections.map((section) => (
                <li key={section}>{SECTION_LABELS[section]}</li>
              ))}
            </ul>
          </div>
        ) : null}
        {brief.missingInformation.length > 0 ? (
          <div>
            <h4 className="font-semibold">Missing information</h4>
            <ul className="mt-1 list-disc pl-5">
              {brief.missingInformation.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </div>
        ) : null}
        {conflicts.length > 0 ? (
          <div>
            <h4 className="font-semibold">Conflicts and gaps</h4>
            <ul className="mt-1 list-disc pl-5">
              {conflicts.map((finding) => (
                <li key={`${finding.text}-${finding.sourceIds.join("-")}`}>
                  {finding.text}{" "}
                  {finding.sourceIds.map((sourceId) => (
                    <SourceCitationLink key={sourceId} sourceId={sourceId} />
                  ))}
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        {brief.dissent.length > 0 ? (
          <div>
            <h4 className="font-semibold">Dissent retained</h4>
            <ul className="mt-1 list-disc pl-5">
              {brief.dissent.map((item) => (
                <li key={item}>{item}</li>
              ))}
            </ul>
          </div>
        ) : null}
      </AlertDescription>
    </Alert>
  );
}

function MainBriefSections({
  published,
}: {
  published: PublishedCommandBrief;
}) {
  const { brief } = published;
  return (
    <div className="space-y-4" data-testid="brief-main-sections">
      <BriefSectionCard
        findings={brief.sections.decisions}
        prominent
        section="decisions"
      />
      <BriefSectionCard findings={brief.sections.today} section="today" />
      <WatchItems published={published} />
      <div className="grid gap-4 lg:grid-cols-2">
        <BriefSectionCard
          findings={brief.sections.operations}
          section="operations"
        />
        <BriefSectionCard
          findings={brief.sections.intelligence}
          section="intelligence"
        />
        <BriefSectionCard
          findings={brief.sections.logistics}
          section="logistics"
        />
        <BriefSectionCard
          findings={brief.sections.navigation}
          section="navigation"
        />
        <BriefSectionCard
          findings={brief.sections.daily_routine}
          section="daily_routine"
        />
        <BriefSectionCard findings={brief.sections.reports} section="reports" />
      </div>
      <BriefSectionCard
        findings={brief.sections.planning_30_60_90}
        section="planning_30_60_90"
      />
    </div>
  );
}

export type DailyCommandBriefProps = {
  readonly status: BriefRunStatus | null;
  readonly history: readonly BriefRunStatus[];
  readonly latest: PublishedCommandBrief | null;
  readonly schedule: BriefSchedule | null;
  readonly systemStatus: CommandConsoleStatusViewModel;
  readonly loading: boolean;
  readonly busy: boolean;
  readonly error: string | null;
  readonly onGenerate: () => void;
  readonly onCancel: () => void;
  readonly onScheduleChange: (patch: CommandBriefSchedulePatch) => void;
};

export function DailyCommandBrief({
  status,
  history,
  latest,
  schedule,
  systemStatus,
  loading,
  busy,
  error,
  onGenerate,
  onCancel,
  onScheduleChange,
}: DailyCommandBriefProps) {
  const visibleStatus =
    latest && (status?.state === "completed" || status?.state === "degraded")
      ? null
      : status;

  return (
    <section
      aria-labelledby="daily-command-brief-heading"
      className="space-y-5"
      data-testid="daily-command-brief"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p className="text-xs font-semibold uppercase tracking-widest text-[#d8aa4f]">
            Quarterdeck brief
          </p>
          <h2
            className="mt-1 text-xl font-semibold"
            id="daily-command-brief-heading"
          >
            Daily Command Brief
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Decisions, priorities and forward planning from your configured
            command sources.
          </p>
        </div>
        <Button
          disabled={busy || ACTIVE_STATES.has(status?.state ?? "completed")}
          onClick={onGenerate}
          type="button"
        >
          {loading ? (
            <RefreshCw
              aria-hidden="true"
              className="motion-safe:animate-spin"
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

      <BriefStatus status={visibleStatus} busy={busy} onCancel={onCancel} />

      {!latest ? (
        <Card>
          <CardContent className="py-8 text-center">
            <p className="font-medium">
              No Daily Command Brief has been generated.
            </p>
            <p className="mt-1 text-sm text-muted-foreground">
              Generate a brief to assemble the latest available RAG, Memory,
              World Monitor, Calendar, Reminders, Notes and selected-file
              inputs.
            </p>
          </CardContent>
        </Card>
      ) : (
        <MainBriefSections published={latest} />
      )}

      <BriefEvidenceDisclosure
        history={history}
        published={latest}
        status={status}
        systemStatus={systemStatus}
      />

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

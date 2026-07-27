import { ChevronDown, ShieldAlert } from "lucide-react";

import type {
  BriefRunStatus,
  PublishedCommandBrief,
} from "@/features/command-console/domain/briefContracts";
import type { CommandConsoleStatusViewModel } from "@/features/command-console/hooks/useCommandConsoleStatus";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Badge } from "@/shared/ui/badge";
import { Card, CardContent } from "@/shared/ui/card";

import { AdviserContributionCard } from "./AdviserContributionCard";
import { STATE_LABELS } from "./briefPresentation";
import { BriefSectionCard } from "./BriefSectionCard";
import { CommandSystemStatus } from "./CommandSystemStatus";
import { SourceLedger } from "./SourceLedger";

function LifecycleHistory({ history }: { history: readonly BriefRunStatus[] }) {
  if (history.length === 0) return null;

  return (
    <section aria-labelledby="brief-lifecycle-history-heading">
      <h3
        className="mb-3 text-base font-semibold"
        id="brief-lifecycle-history-heading"
      >
        Lifecycle history
      </h3>
      <ol className="grid gap-2 sm:grid-cols-2">
        {history.map((entry) => (
          <li
            className="flex items-center justify-between gap-3 rounded-lg border p-3 text-sm"
            key={`${entry.runId}-${entry.sequence}`}
          >
            <span>{STATE_LABELS[entry.state]}</span>
            <time className="text-muted-foreground" dateTime={entry.updatedAt}>
              {entry.updatedAt}
            </time>
          </li>
        ))}
      </ol>
    </section>
  );
}

function PublicationMetadata({
  published,
}: {
  published: PublishedCommandBrief;
}) {
  return (
    <Card>
      <CardContent className="flex flex-wrap items-start justify-between gap-4 py-4">
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 text-sm">
          <dt className="text-muted-foreground">Generated</dt>
          <dd>
            <time dateTime={published.brief.generatedAt}>
              {published.brief.generatedAt}
            </time>
          </dd>
          <dt className="text-muted-foreground">Snapshot</dt>
          <dd className="break-all font-mono">{published.brief.snapshotId}</dd>
          <dt className="text-muted-foreground">Audit event</dt>
          <dd className="break-all font-mono">
            {published.lifecycleAuditEventId}
          </dd>
        </dl>
        <Badge
          variant={
            published.publicationState === "published" ? "success" : "warning"
          }
        >
          {published.publicationState === "published"
            ? "Signed and published"
            : "Relay publication queued — offline capable"}
        </Badge>
      </CardContent>
    </Card>
  );
}

function PublishedEvidence({
  published,
}: {
  published: PublishedCommandBrief;
}) {
  return (
    <>
      <PublicationMetadata published={published} />

      <Alert>
        <AlertTitle className="flex items-center gap-2">
          <ShieldAlert aria-hidden="true" className="h-4 w-4" />
          Advisory limitation
        </AlertTitle>
        <AlertDescription>
          {published.brief.advisoryLimitation}
        </AlertDescription>
      </Alert>

      <section aria-labelledby="adviser-contributions-heading">
        <h3
          className="mb-3 text-base font-semibold"
          id="adviser-contributions-heading"
        >
          Specialist adviser contributions
        </h3>
        <div className="grid gap-4 lg:grid-cols-2">
          {published.brief.contributions.map((contribution) => (
            <AdviserContributionCard
              contribution={contribution}
              key={contribution.adviser}
            />
          ))}
        </div>
      </section>

      {published.brief.sections.sources.length > 0 ? (
        <BriefSectionCard
          findings={published.brief.sections.sources}
          section="sources"
        />
      ) : null}

      <SourceLedger
        entries={published.brief.sourceLedger}
        freshness={published.brief.sourceFreshness}
      />
    </>
  );
}

export function BriefEvidenceDisclosure({
  history,
  published,
  status,
  systemStatus,
}: {
  history: readonly BriefRunStatus[];
  published: PublishedCommandBrief | null;
  status: BriefRunStatus | null;
  systemStatus: CommandConsoleStatusViewModel;
}) {
  return (
    <details
      className="group overflow-hidden rounded-xl border border-border/70 bg-card/60"
      data-testid="brief-evidence-disclosure"
    >
      <summary className="flex cursor-pointer list-none items-center justify-between gap-4 px-5 py-4 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring">
        <div>
          <span className="font-semibold">Evidence and system status</span>
          <span className="mt-1 block text-sm text-muted-foreground">
            Citations, adviser detail, run history and connector health
          </span>
        </div>
        <div className="flex items-center gap-3">
          {status ? (
            <Badge variant="secondary">{STATE_LABELS[status.state]}</Badge>
          ) : null}
          <ChevronDown
            aria-hidden="true"
            className="h-5 w-5 text-muted-foreground transition-transform group-open:rotate-180 motion-reduce:transition-none"
          />
        </div>
      </summary>
      <div className="space-y-6 border-t border-border/70 p-5">
        {published ? (
          <PublishedEvidence published={published} />
        ) : (
          <p className="rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
            Source evidence will appear here after the first brief is generated.
          </p>
        )}
        <LifecycleHistory history={history} />
        <CommandSystemStatus status={systemStatus} />
      </div>
    </details>
  );
}

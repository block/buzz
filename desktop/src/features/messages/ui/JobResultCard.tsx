import type { LucideIcon } from "lucide-react";
import {
  Ban,
  Boxes,
  CheckCircle2,
  CircleAlert,
  CircleDotDashed,
  CircleX,
  ExternalLink,
  FileText,
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  Hammer,
  Image,
  Link2,
  ListChecks,
  PackageCheck,
  Rocket,
  ScrollText,
  Workflow,
  XCircle,
} from "lucide-react";

import {
  getJobArtifactKindLabel,
  getJobResultDispositionLabel,
  type JobArtifactKind,
  type JobResult,
  type JobResultDisposition,
  type JobVerificationStatus,
} from "@/features/messages/lib/jobResult";
import { cn } from "@/shared/lib/cn";
import { isSafeUrl } from "@/shared/lib/url";

const dispositionPresentation: Record<
  JobResultDisposition,
  {
    icon: LucideIcon;
    cardClass: string;
    iconClass: string;
    badgeClass: string;
  }
> = {
  completed: {
    icon: CheckCircle2,
    cardClass: "border-emerald-500/35 bg-emerald-500/5",
    iconClass: "text-emerald-500",
    badgeClass:
      "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
  },
  partial: {
    icon: CircleAlert,
    cardClass: "border-amber-500/35 bg-amber-500/5",
    iconClass: "text-amber-500",
    badgeClass:
      "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400",
  },
  blocked: {
    icon: Ban,
    cardClass: "border-orange-500/35 bg-orange-500/5",
    iconClass: "text-orange-500",
    badgeClass:
      "border-orange-500/30 bg-orange-500/10 text-orange-600 dark:text-orange-400",
  },
  failed: {
    icon: XCircle,
    cardClass: "border-destructive/35 bg-destructive/5",
    iconClass: "text-destructive",
    badgeClass: "border-destructive/30 bg-destructive/10 text-destructive",
  },
  no_artifact: {
    icon: PackageCheck,
    cardClass: "border-sky-500/35 bg-sky-500/5",
    iconClass: "text-sky-500",
    badgeClass:
      "border-sky-500/30 bg-sky-500/10 text-sky-600 dark:text-sky-400",
  },
};

const artifactIcons: Record<JobArtifactKind, LucideIcon> = {
  file: FileText,
  media: Image,
  branch: GitBranch,
  commit: GitCommitHorizontal,
  pull_request: GitPullRequest,
  canvas: ScrollText,
  workflow_output: Workflow,
  build: Hammer,
  deployment: Rocket,
  link: Link2,
  other: Boxes,
};

const verificationPresentation: Record<
  JobVerificationStatus,
  { icon: LucideIcon; label: string; className: string }
> = {
  passed: {
    icon: CheckCircle2,
    label: "Passed",
    className: "text-emerald-500",
  },
  failed: {
    icon: CircleX,
    label: "Failed",
    className: "text-destructive",
  },
  not_run: {
    icon: CircleDotDashed,
    label: "Not run",
    className: "text-muted-foreground",
  },
};

export function JobResultCard({ result }: { result: JobResult }) {
  const disposition = dispositionPresentation[result.disposition];
  const DispositionIcon = disposition.icon;

  return (
    <article
      aria-label="Agent job handoff"
      className={cn(
        "overflow-hidden rounded-2xl border bg-card/70 text-sm shadow-xs",
        disposition.cardClass,
      )}
      data-disposition={result.disposition}
      data-testid="job-result-card"
    >
      <header className="flex items-start gap-3 border-b border-border/50 bg-background/45 px-3 py-3">
        <span
          aria-hidden
          className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-background/70"
        >
          <DispositionIcon className={cn("h-4 w-4", disposition.iconClass)} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              Agent handoff
            </p>
            <span
              className={cn(
                "rounded-md border px-1.5 py-0.5 text-2xs font-medium",
                disposition.badgeClass,
              )}
            >
              {getJobResultDispositionLabel(result.disposition)}
            </span>
          </div>
          <p className="mt-1 text-sm font-semibold leading-snug text-foreground">
            {result.outcome}
          </p>
        </div>
      </header>

      <div className="space-y-3 px-3 py-3">
        <section aria-label="Requested outcome">
          <h4 className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
            Requested outcome
          </h4>
          <p className="mt-1 leading-relaxed text-foreground/90">
            {result.requestedOutcome}
          </p>
        </section>

        {result.lastProgress ? (
          <section className="rounded-lg border border-border/55 bg-background/40 px-2.5 py-2">
            <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Last progress
            </p>
            <p className="mt-1 leading-relaxed text-foreground/85">
              {result.lastProgress}
            </p>
          </section>
        ) : null}

        {result.blocker ? (
          <section className="rounded-lg border border-orange-500/30 bg-orange-500/10 px-2.5 py-2">
            <p className="flex items-center gap-1.5 text-2xs font-semibold uppercase tracking-wide text-orange-600 dark:text-orange-400">
              <CircleAlert aria-hidden className="h-3.5 w-3.5" />
              Blocker
            </p>
            <p className="mt-1 leading-relaxed text-foreground/90">
              {result.blocker}
            </p>
          </section>
        ) : null}

        <section aria-label="Artifacts">
          <div className="flex items-center gap-1.5">
            <Boxes aria-hidden className="h-3.5 w-3.5 text-muted-foreground" />
            <h4 className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Artifacts
            </h4>
            <span className="text-2xs text-muted-foreground/70">
              {result.artifacts.length}
            </span>
          </div>

          {result.artifacts.length > 0 ? (
            <ul className="mt-1.5 space-y-1.5">
              {result.artifacts.map((artifact) => {
                const ArtifactIcon = artifactIcons[artifact.kind];
                const linkedReference = isSafeUrl(artifact.reference);

                return (
                  <li
                    className="flex min-w-0 items-start gap-2 rounded-lg border border-border/50 bg-background/45 px-2.5 py-2"
                    key={`${artifact.kind}-${artifact.reference}-${artifact.label}-${artifact.sourceState ?? ""}`}
                  >
                    <ArtifactIcon
                      aria-hidden
                      className="mt-0.5 h-4 w-4 shrink-0 text-primary/80"
                    />
                    <div className="min-w-0 flex-1">
                      <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
                        <p className="font-medium text-foreground">
                          {artifact.label}
                        </p>
                        <span className="text-2xs text-muted-foreground">
                          {getJobArtifactKindLabel(artifact.kind)}
                        </span>
                      </div>
                      {linkedReference ? (
                        <a
                          className="mt-0.5 inline-flex max-w-full items-center gap-1 break-all font-mono text-xs text-primary hover:underline focus-visible:rounded-sm focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                          href={artifact.reference}
                          rel="noreferrer noopener"
                          target="_blank"
                        >
                          <span className="min-w-0">{artifact.reference}</span>
                          <ExternalLink
                            aria-hidden
                            className="h-3 w-3 shrink-0"
                          />
                        </a>
                      ) : (
                        <p className="mt-0.5 break-all font-mono text-xs text-muted-foreground">
                          {artifact.reference}
                        </p>
                      )}
                      {artifact.sourceState ? (
                        <p className="mt-1 break-all text-2xs text-muted-foreground/80">
                          Source state:{" "}
                          <span className="font-mono">
                            {artifact.sourceState}
                          </span>
                        </p>
                      ) : null}
                    </div>
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="mt-1.5 rounded-lg border border-dashed border-border/60 px-2.5 py-2 text-xs text-muted-foreground">
              {result.disposition === "no_artifact"
                ? "No durable artifact was expected for this result."
                : "No artifact was reported for this result."}
            </p>
          )}
        </section>

        <section aria-label="Verification">
          <div className="flex items-center gap-1.5">
            <ListChecks
              aria-hidden
              className="h-3.5 w-3.5 text-muted-foreground"
            />
            <h4 className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Verification
            </h4>
            <span className="text-2xs text-muted-foreground/70">
              {result.verification.length}
            </span>
          </div>

          {result.verification.length > 0 ? (
            <ul className="mt-1.5 divide-y divide-border/45 rounded-lg border border-border/50 bg-background/45">
              {result.verification.map((verification) => {
                const presentation =
                  verificationPresentation[verification.status];
                const VerificationIcon = presentation.icon;

                return (
                  <li
                    className="flex items-start gap-2 px-2.5 py-2"
                    key={`${verification.label}-${verification.status}-${verification.evidence ?? ""}`}
                  >
                    <VerificationIcon
                      aria-hidden
                      className={cn(
                        "mt-0.5 h-4 w-4 shrink-0",
                        presentation.className,
                      )}
                    />
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-baseline gap-x-2">
                        <p className="font-medium text-foreground">
                          {verification.label}
                        </p>
                        <span
                          className={cn(
                            "text-2xs font-medium",
                            presentation.className,
                          )}
                        >
                          {presentation.label}
                        </span>
                      </div>
                      {verification.evidence ? (
                        <p className="mt-0.5 whitespace-pre-wrap break-words text-xs text-muted-foreground">
                          {verification.evidence}
                        </p>
                      ) : null}
                    </div>
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="mt-1.5 rounded-lg border border-dashed border-border/60 px-2.5 py-2 text-xs text-muted-foreground">
              No verification was reported.
            </p>
          )}
        </section>
      </div>
    </article>
  );
}

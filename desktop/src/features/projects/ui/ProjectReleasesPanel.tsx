import { TriangleAlert } from "lucide-react";
import type * as React from "react";

import type { Repository as Project } from "@/features/projects/hooks";
import { useProjectBwSnapshotQuery } from "@/features/projects/hooks";
import {
  bwEvidenceReasonLabel,
  bwReleasesView,
  type BwArtifactView,
  type BwHandoffView,
  type BwIncompleteEvidence,
  type BwReleaseSetView,
  type BwReleasesView,
} from "@/features/projects/bwReleases";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import { cn } from "@/shared/lib/cn";
import { PubKey } from "@/shared/ui/PubKey";
import { PROJECT_DETAIL_PANEL_MESSAGE_CLASS } from "./projectPanelStyles";

// P5_6A: read-only release/run/artifact views over the accepted Core
// projection (`get_project_bw`). This panel renders only what Core decided:
// set status from `projection.sets`, verdicts from `projection.artifact_verdicts`,
// the current handoff from `projection.handoffs`, details from accepted
// records, and pending/refused chains as traceable references. It contains no
// side-effecting control and never upgrades a provider success to completed.

const STATUS_CLASS: Record<string, string> = {
  active: "text-blue-500",
  completed: "text-purple-400",
  failed: "text-red-500",
  aborted: "text-amber-500",
};

const VERDICT_CLASS: Record<string, string> = {
  accepted: "text-green-500",
  rejected: "text-red-500",
  conflict: "text-amber-500",
  unreviewed: "text-muted-foreground",
};

const OUTCOME_CLASS: Record<string, string> = {
  pending: "text-muted-foreground",
  reject: "text-red-500",
  conflict: "text-amber-500",
};

function shortId(id: string): string {
  return id.slice(0, 12);
}

function formatBytes(bytes: number | null): string {
  if (bytes === null) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function MetaCell({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="min-w-0">
      <div className="text-2xs uppercase tracking-wide text-muted-foreground">
        {label}
      </div>
      <div className="min-w-0 truncate text-xs">{children}</div>
    </div>
  );
}

function MonoValue({ value }: { value: string | null }) {
  return (
    <span className="font-mono" title={value ?? undefined}>
      {value ?? "—"}
    </span>
  );
}

function MemberRows({ set }: { set: BwReleaseSetView }) {
  if (set.members.length === 0) {
    return <p className="text-xs text-muted-foreground">No frozen members.</p>;
  }
  return (
    <ul className="space-y-1">
      {set.members.map((member) => (
        <li
          className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs"
          data-testid="bw-release-member"
          key={member.issue}
        >
          <span className="font-mono text-2xs">
            ISS-{member.issue.slice(0, 8).toUpperCase()}
          </span>
          <span className="min-w-0 truncate">{member.title ?? "Issue"}</span>
          {/* Historical issue acceptance — deliberately separate from any
              artifact verdict below. */}
          <span
            className="text-muted-foreground"
            data-testid="bw-release-member-state"
          >
            {member.state ?? "unknown state"}
          </span>
          {member.implemented ? (
            <span
              className="font-mono text-2xs text-muted-foreground"
              title={`Implemented transition ${member.implemented}`}
            >
              impl:{shortId(member.implemented)}
            </span>
          ) : null}
        </li>
      ))}
    </ul>
  );
}

function RunRows({ set }: { set: BwReleaseSetView }) {
  if (set.requests.length === 0) {
    return (
      <p
        className="text-xs text-muted-foreground"
        data-testid="bw-release-no-request"
      >
        No build request recorded for this set.
      </p>
    );
  }
  return (
    <div className="space-y-1.5">
      {set.requests.map((request) => {
        const runs = set.runs.filter((run) => run.request === request.id);
        return (
          <div className="space-y-1" key={request.id}>
            <div className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
              <span data-testid="bw-release-request">
                Request attempt {request.attempt ?? "?"}
              </span>
              {request.retryOf ? (
                <span className="font-mono text-2xs">
                  retry of run:{shortId(request.retryOf)}
                </span>
              ) : null}
              {request.worker ? (
                <span className="inline-flex items-center gap-1">
                  worker <PubKey interactive={false} pubkey={request.worker} />
                </span>
              ) : null}
            </div>
            {runs.length === 0 ? (
              <p
                className="text-xs text-muted-foreground"
                data-testid="bw-release-run-missing"
              >
                No accepted run for this request — external CI evidence is
                pending or missing.
              </p>
            ) : null}
            {runs.map((run) => (
              <div
                className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs"
                data-testid="bw-release-run"
                key={run.id}
              >
                <span
                  className={cn(
                    "font-medium",
                    run.result === "success"
                      ? "text-green-500"
                      : run.result === "failure"
                        ? "text-red-500"
                        : "text-muted-foreground",
                  )}
                  data-testid="bw-release-run-result"
                >
                  {run.result ?? "no result"}
                </span>
                <span className="text-muted-foreground">
                  run {run.attempt ?? "?"}
                </span>
                {run.provider ? <span>{run.provider}</span> : null}
                {run.runId ? (
                  <span className="font-mono text-2xs">{run.runId}</span>
                ) : null}
                {run.url ? (
                  <span
                    className="min-w-0 truncate text-muted-foreground"
                    title={run.url}
                  >
                    {run.url}
                  </span>
                ) : null}
                {run.head ? (
                  <span
                    className="font-mono text-2xs text-muted-foreground"
                    title={run.head}
                  >
                    head:{shortId(run.head)}
                  </span>
                ) : null}
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}

function ArtifactRows({ set }: { set: BwReleaseSetView }) {
  if (set.artifacts.length === 0) {
    return (
      <p
        className="text-xs text-muted-foreground"
        data-testid="bw-release-no-artifact"
      >
        No accepted artifact for this set.
      </p>
    );
  }
  return (
    <ul className="space-y-2">
      {set.artifacts.map((artifact: BwArtifactView) => (
        <li
          className="space-y-1 rounded-lg border border-border/40 p-2"
          data-testid="bw-release-artifact"
          key={artifact.id}
        >
          <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs">
            <span className="font-mono text-2xs">
              artifact:{shortId(artifact.id)}
            </span>
            <span className="text-muted-foreground">
              {artifact.mime ?? "unknown type"}
            </span>
            <span className="text-muted-foreground">
              {formatBytes(artifact.bytes)}
            </span>
          </div>
          <div className="text-xs">
            <span className="text-muted-foreground">SHA-256 </span>
            <span
              className="break-all font-mono text-2xs"
              data-testid="bw-release-artifact-sha256"
            >
              {artifact.sha256 ?? "—"}
            </span>
          </div>
          {artifact.url ? (
            <div
              className="min-w-0 truncate text-xs text-muted-foreground"
              title={artifact.url}
            >
              {artifact.url}
            </div>
          ) : null}
          <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs">
            {set.members.map((member) => {
              const verdict = artifact.verdicts[member.issue] ?? "unreviewed";
              return (
                <span
                  className={cn("font-medium", VERDICT_CLASS[verdict])}
                  data-testid="bw-release-artifact-verdict"
                  key={member.issue}
                >
                  ISS-{member.issue.slice(0, 8).toUpperCase()}: {verdict}
                </span>
              );
            })}
          </div>
        </li>
      ))}
    </ul>
  );
}

function HandoffSection({ handoff }: { handoff: BwHandoffView | null }) {
  if (!handoff) {
    return (
      <p
        className="text-xs text-muted-foreground"
        data-testid="bw-release-no-handoff"
      >
        No accepted test-ready handoff for this set.
      </p>
    );
  }
  const installation = handoff.installation;
  return (
    <div className="space-y-1.5" data-testid="bw-release-handoff">
      <div className="flex flex-wrap items-center gap-x-2 text-xs">
        <span className="text-muted-foreground">Tester</span>
        {handoff.tester ? (
          <span data-testid="bw-release-handoff-tester">
            <PubKey interactive={false} pubkey={handoff.tester} />
          </span>
        ) : (
          <span className="text-muted-foreground">not named</span>
        )}
      </div>
      {installation ? (
        <div className="space-y-1 text-xs">
          <div className="min-w-0 truncate">
            <span className="text-muted-foreground">Download </span>
            <span title={installation.url ?? undefined}>
              {installation.url ?? "—"}
            </span>
          </div>
          <div className="flex flex-wrap items-center gap-x-2 text-muted-foreground">
            {installation.method ? <span>{installation.method}</span> : null}
            <span data-testid="bw-release-handoff-immutable">
              {installation.immutable ? "immutable" : "not immutable"}
            </span>
          </div>
          {installation.unsigned ? (
            <p
              className="flex items-center gap-1.5 text-amber-600 dark:text-amber-400"
              data-testid="bw-release-handoff-unsigned"
            >
              <TriangleAlert className="h-3.5 w-3.5 shrink-0" />
              Unsigned installer — no signature trust; verify the SHA-256 before
              running.
            </p>
          ) : null}
          {installation.instructions ? (
            <p className="text-muted-foreground">{installation.instructions}</p>
          ) : null}
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          Installation details missing from the handoff record.
        </p>
      )}
      {handoff.limitations ? (
        <p className="text-xs text-muted-foreground">{handoff.limitations}</p>
      ) : null}
    </div>
  );
}

function ReleaseSetCard({ set }: { set: BwReleaseSetView }) {
  return (
    <div
      className="space-y-3 rounded-xl border border-border/60 p-3"
      data-testid="bw-release-set"
    >
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
        <h3 className="min-w-0 truncate text-sm font-semibold">
          {set.release ?? `Release set ${shortId(set.id)}`}
        </h3>
        <span
          className={cn(
            "text-xs font-medium capitalize",
            STATUS_CLASS[set.status],
          )}
          data-testid="bw-release-set-status"
        >
          {set.status}
        </span>
        {set.platform ? (
          <span className="text-xs text-muted-foreground">{set.platform}</span>
        ) : null}
        {set.stream ? (
          <span className="font-mono text-2xs text-muted-foreground">
            {set.stream}
          </span>
        ) : null}
        {set.frozenAt !== null ? (
          <span className="text-xs text-muted-foreground">
            frozen {relativeTime(set.frozenAt)}
          </span>
        ) : null}
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <MetaCell label="Freeze SHA">
          <span data-testid="bw-release-freeze-sha">
            <MonoValue value={set.freezeSha} />
          </span>
        </MetaCell>
        <MetaCell label="Pipeline">
          {set.pipeline ? (
            <span data-testid="bw-release-pipeline">
              {set.pipeline.workflow ?? "workflow unknown"}
              {set.pipeline.provider ? ` · ${set.pipeline.provider}` : ""}
            </span>
          ) : (
            <span className="text-muted-foreground">
              pipeline record missing
            </span>
          )}
        </MetaCell>
      </div>
      <section className="space-y-1">
        <h4 className="text-xs font-semibold text-muted-foreground">Members</h4>
        <MemberRows set={set} />
      </section>
      <section className="space-y-1">
        <h4 className="text-xs font-semibold text-muted-foreground">
          Requests and runs
        </h4>
        <RunRows set={set} />
      </section>
      <section className="space-y-1">
        <h4 className="text-xs font-semibold text-muted-foreground">
          Artifacts
        </h4>
        <ArtifactRows set={set} />
      </section>
      <section className="space-y-1">
        <h4 className="text-xs font-semibold text-muted-foreground">
          Test-ready handoff
        </h4>
        <HandoffSection handoff={set.handoff} />
      </section>
    </div>
  );
}

function IncompleteEvidenceSection({ view }: { view: BwReleasesView }) {
  if (view.incomplete.length === 0) return null;
  return (
    <div
      className="space-y-1.5 rounded-xl border border-border/60 p-3"
      data-testid="bw-release-incomplete"
    >
      <h4 className="text-xs font-semibold text-muted-foreground">
        Incomplete or refused evidence
      </h4>
      <p className="text-xs text-muted-foreground">
        Missing external evidence stays pending — a signed claim alone proves
        nothing. Each entry references the exact event Core did not accept.
      </p>
      <ul className="space-y-1">
        {view.incomplete.map((entry: BwIncompleteEvidence) => (
          <li
            className="flex min-w-0 flex-wrap items-center gap-x-2 text-xs"
            data-testid="bw-release-incomplete-entry"
            key={entry.eventId}
          >
            <span className="font-mono text-2xs" title={entry.eventId}>
              {shortId(entry.eventId)}
            </span>
            <span
              className={cn(
                "font-medium capitalize",
                OUTCOME_CLASS[entry.outcome],
              )}
            >
              {entry.outcome}
            </span>
            <span className="text-muted-foreground">
              {bwEvidenceReasonLabel(entry)}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function ProjectReleasesPanel({ project }: { project: Project }) {
  const { data, error, isLoading } = useProjectBwSnapshotQuery(project);
  if (isLoading) {
    return (
      <div className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}>
        Loading release state…
      </div>
    );
  }
  if (error || !data) {
    return (
      <div
        className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}
        data-testid="bw-releases-error"
      >
        Release history unavailable
        {error instanceof Error && error.message ? `: ${error.message}` : "."}
      </div>
    );
  }
  if (!data.activation) {
    return (
      <div
        className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}
        data-testid="bw-releases-inactive"
      >
        BW release tracking is not activated for this repository.
      </div>
    );
  }
  const view = bwReleasesView(data);
  return (
    <div className="space-y-3" data-testid="bw-releases-panel">
      {view.building ? (
        <p
          className="text-xs text-muted-foreground"
          data-testid="bw-release-building"
        >
          A build request is awaiting its run result.
        </p>
      ) : null}
      {view.sets.length === 0 ? (
        <div
          className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}
          data-testid="bw-releases-empty"
        >
          No release set has been frozen yet.
        </div>
      ) : (
        view.sets.map((set) => <ReleaseSetCard key={set.id} set={set} />)
      )}
      <IncompleteEvidenceSection view={view} />
    </div>
  );
}

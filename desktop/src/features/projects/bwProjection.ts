import type { Project } from "./hooks";
import { fetchProjectsWorkItems } from "./projectWorkItems";
import { invokeTauri } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { eventToProjectIssue, type ProjectIssue } from "./projectIssues.mjs";

export type BwNotice = {
  event_id: string;
  outcome: string;
  stage: string;
  code: string;
};
export type BwSnapshot = {
  repo: string;
  activation: { policy: string; genesis: string } | null;
  projection: {
    issues: Record<string, string>;
    issue_fields: Record<
      string,
      {
        title: string;
        description: string;
        acceptance_criteria: string[];
        non_goals: string[];
        type: string;
        platform: string | null;
        priority: string;
      }
    >;
    conflicts: string[];
    children: Record<string, string[]>;
    relations: unknown[];
    artifact_verdicts: Record<string, unknown>;
  };
  records: Record<string, RelayEvent>;
  decisions: Record<string, { outcome: string; stage: string; code: string }>;
  notices: Record<string, BwNotice[]>;
};
export type BwIssueView = {
  state: string | null;
  enrolled: boolean;
  nextActor: string;
  notices: BwNotice[];
  snapshot: BwSnapshot;
  /** A concurrent text/acceptance-criteria edit produced a visible technical
   * fork for this issue. Never auto-resolved; the UI must show it. */
  fieldConflict: boolean;
};
const labels: Record<string, ProjectIssue["status"]> = {
  triage: "Triage",
  backlog: "Backlog",
  ready: "Backlog",
  "in-development": "In Development",
  implemented: "Implemented",
  resolved: "Done",
  rework: "Backlog",
  closed: "Closed",
};

function bwTag(event: RelayEvent, name: string): string | undefined {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

export type BwChainRecordType = "issue-update" | "triage-action";

/** The current head of a per-issue BW chain (issue-update or triage-action),
 * computed only from accepted records the snapshot already returned. A
 * technical fork — two accepted records with no successor — is reported as
 * a conflict instead of picking either one as `previous`. */
export function bwChainHead(
  snapshot: BwSnapshot,
  issueId: string,
  recordType: BwChainRecordType,
): { headId: string | null; conflict: boolean } {
  const records = Object.values(snapshot.records).filter(
    (event) =>
      event.kind === 46100 &&
      bwTag(event, "record") === recordType &&
      bwTag(event, "issue") === issueId,
  );
  const referenced = new Set(
    records
      .map((event) => bwTag(event, "previous"))
      .filter((value): value is string => Boolean(value)),
  );
  const heads = records.filter((event) => !referenced.has(event.id));
  if (heads.length === 0) return { headId: null, conflict: false };
  if (heads.length > 1) return { headId: null, conflict: true };
  return { headId: heads[0].id, conflict: false };
}

/** True when any accepted issue-update for this issue is part of a visible
 * technical conflict (a fork in `snapshot.projection.conflicts`). */
export function bwFieldConflict(
  snapshot: BwSnapshot,
  issueId: string,
): boolean {
  const conflictIds = new Set(snapshot.projection.conflicts);
  return Object.values(snapshot.records).some(
    (event) =>
      event.kind === 46100 &&
      bwTag(event, "record") === "issue-update" &&
      bwTag(event, "issue") === issueId &&
      conflictIds.has(event.id),
  );
}

/** The content of the current triage-action head for an issue, if any and
 * unconflicted. Used only to distinguish "new" (no triage action yet) from
 * "awaiting clarification" (need-info is the current head) within the
 * single `triage` workflow state — Core itself does not project that
 * distinction as a separate state. */
function currentTriageAction(
  snapshot: BwSnapshot,
  issueId: string,
): string | null {
  const { headId, conflict } = bwChainHead(snapshot, issueId, "triage-action");
  if (conflict || !headId) return null;
  const head = snapshot.records[headId];
  if (!head) return null;
  try {
    return (JSON.parse(head.content) as { action?: string }).action ?? null;
  } catch {
    return null;
  }
}

/** Presentation only: Core owns enrollment, state, conflicts and historical acceptance. */
export function mergeBwIssues(
  legacy: ProjectIssue[],
  snapshot: BwSnapshot,
): ProjectIssue[] {
  const byId = new Map(legacy.map((issue) => [issue.id, issue]));
  const ids = new Set([
    ...Object.keys(snapshot.projection.issues),
    ...Object.keys(snapshot.notices),
    ...Object.keys(snapshot.records).filter(
      (id) => snapshot.records[id]?.kind === 1621,
    ),
  ]);
  for (const id of ids) {
    const root = snapshot.records[id];
    if (root?.kind !== 1621) continue;
    const issue = byId.get(id) ?? eventToProjectIssue(root);
    const state = snapshot.projection.issues[id] ?? null;
    const fields = snapshot.projection.issue_fields[id];
    const notices = snapshot.notices[id] ?? [];
    const enrolled = state !== null;
    const needsClarification =
      state === "triage" && currentTriageAction(snapshot, id) === "need-info";
    const status = needsClarification
      ? "Needs Clarification"
      : state
        ? (labels[state] ?? "Triage")
        : "Triage";
    byId.set(id, {
      ...issue,
      title: fields?.title ?? issue.title,
      content: fields?.description ?? issue.content,
      status,
      workflowStatus: null,
      currentReview: null,
      assignees: [],
      assigneeOperationHeads: {},
      bw: {
        state,
        enrolled,
        notices,
        snapshot,
        fieldConflict: bwFieldConflict(snapshot, id),
        nextActor: notices.length
          ? "Waiting for verified history or evidence"
          : !enrolled
            ? "Not yet a BW issue — enroll to start tracking"
            : state === "resolved" || state === "closed"
              ? "No action required"
              : needsClarification
                ? "Waiting on the requested clarification"
                : state === "in-development" || state === "ready"
                  ? "Selected writer"
                  : state === "implemented"
                    ? "Release coordinator"
                    : "Triage operator",
      },
    });
  }
  return [...byId.values()].sort(
    (a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id),
  );
}

export function fetchBwSnapshot(repo: string): Promise<BwSnapshot> {
  return invokeTauri("get_project_bw", { repo });
}

/** Use the same Core view for cross-project lists and repository details. */
export async function fetchBwWorkItems(projects: Project[]) {
  const result = await fetchProjectsWorkItems(projects);
  const contexts = new Map(
    projects.flatMap((project) =>
      project.repositories.map(
        (repository) =>
          [repository.repoAddress, { project, repository }] as const,
      ),
    ),
  );
  const groups = await Promise.all(
    [...contexts.entries()].map(async ([repo, context]) => {
      const snapshot = await fetchBwSnapshot(repo);
      const legacy = result.issues.items
        .filter((row) => row.repository.repoAddress === repo)
        .map((row) => row.issue);
      return mergeBwIssues(legacy, snapshot).map((issue) => ({
        ...context,
        issue,
      }));
    }),
  );
  return { ...result, issues: { ...result.issues, items: groups.flat() } };
}

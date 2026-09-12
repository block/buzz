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
/** The current issue-state chain head's own already-accepted content body
 * (P4E), passed through verbatim from Core
 * (`crates/buzz-core/src/bw/projection.rs`) — never re-derived here. Only
 * the fields valid for the record's own `state` are present; see
 * NIP-BW.md "issue-state and the existing assignment wire". */
export type BwIssueStateFields = {
  state: string;
  stream?: string;
  assignment?: string | null;
  update?: string;
  rework?: string | null;
  terminal_set?: string;
  triage?: string;
  commit?: string;
  tests?: string;
  remote_readback?: {
    repo: string;
    stream: string;
    head: string;
    observed_at: number;
  };
};
export type BwRelation = { issue: string; relation: string; target: string };
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
    /** Current issue-state head body, keyed by issue ID — absent when the
     * head is ambiguous (fork) or the issue was never enrolled. */
    issue_state: Record<string, BwIssueStateFields>;
    /** The current issue-state head's own event ID, so a producer can chain
     * the next transition's `previous` tag without re-deriving this head. */
    issue_state_id: Record<string, string>;
    /** Executable-leaf eligibility per issue, decided by Core
     * (`is_executable_leaf`) — never re-derived client-side. */
    leaf: Record<string, boolean>;
    conflicts: string[];
    children: Record<string, string[]>;
    relations: BwRelation[];
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
  ready: "Ready",
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

export type BwAssignmentOperation = "assignment" | "unassignment";
export type BwAssignmentHead = {
  headId: string | null;
  /** The sole delegate an `assignment` head adds; `null` when the head is
   * an `unassignment` (no current writer) or there is no head at all. */
  writer: string | null;
  operation: BwAssignmentOperation | null;
  conflict: boolean;
};

/** The current head of an issue's *existing* kind:1 assignment/unassignment
 * chain (NIP-BW.md "issue-state and the existing assignment wire" — no new
 * record type or kind), computed only from records the snapshot already
 * returned as accepted. Mirrors `bwChainHead`'s fork handling: two accepted
 * heads with no successor is a conflict, never a guessed winner. Because
 * Core's own `roles()`/`causality()` already decided which kind:1 events are
 * historically valid before they ever reached `snapshot.records`, this is
 * display of an already-made decision, not a second authority check. */
export function bwAssignmentHead(
  snapshot: BwSnapshot,
  issueId: string,
): BwAssignmentHead {
  const records = Object.values(snapshot.records).filter((event) => {
    if (event.kind !== 1) return false;
    const operation = bwTag(event, "t");
    return (
      (operation === "assignment" || operation === "unassignment") &&
      bwTag(event, "e") === issueId
    );
  });
  const referenced = new Set(
    records
      .map((event) => bwTag(event, "prior"))
      .filter((value): value is string => Boolean(value)),
  );
  const heads = records.filter((event) => !referenced.has(event.id));
  if (heads.length === 0) {
    return { headId: null, writer: null, operation: null, conflict: false };
  }
  if (heads.length > 1) {
    return { headId: null, writer: null, operation: null, conflict: true };
  }
  const head = heads[0];
  const operation = bwTag(head, "t") as BwAssignmentOperation;
  return {
    headId: head.id,
    writer: operation === "assignment" ? (bwTag(head, "p") ?? null) : null,
    operation,
    conflict: false,
  };
}

/** Resolve the writer identity bound into the current issue-state head.
 * `assignment` is an event id, not a writer pubkey: follow that exact accepted
 * kind:1 assignment instead of using a possibly newer UI selection. Core
 * revalidates the same binding when the writer-signed transition is submitted. */
export function bwBoundWriter(
  snapshot: BwSnapshot,
  issueId: string,
): string | null {
  const assignmentId = snapshot.projection.issue_state[issueId]?.assignment;
  if (!assignmentId) return null;
  const assignment = snapshot.records[assignmentId];
  if (
    assignment?.kind !== 1 ||
    bwTag(assignment, "t") !== "assignment" ||
    bwTag(assignment, "e") !== issueId
  ) {
    return null;
  }
  return bwTag(assignment, "p") ?? null;
}

export type BwRelationTargetCandidate = {
  id: string;
  reference: string;
  state: string;
  title: string;
};

/** Autocomplete candidates for an issue relation. NIP-BW allows any other
 * enrolled issue in the same repository; it does not restrict targets to
 * backlog/ready. `projection.issues` is Core's accepted enrollment/state
 * inventory, so pending/foreign roots never appear here. */
export function bwRelationTargetCandidates(
  snapshot: BwSnapshot,
  currentIssueId: string,
): BwRelationTargetCandidate[] {
  return Object.entries(snapshot.projection.issues)
    .filter(([id]) => id !== currentIssueId)
    .map(([id, state]) => {
      const root = snapshot.records[id];
      const rootTitle = root?.tags.find((tag) => tag[0] === "subject")?.[1];
      return {
        id,
        reference: `ISS-${id.slice(0, 8).toUpperCase()}`,
        state,
        title:
          snapshot.projection.issue_fields[id]?.title || rootTitle || "Issue",
      };
    })
    .sort((left, right) => left.reference.localeCompare(right.reference));
}

/** Match issue autocomplete text against its user-facing number, full event
 * id, title and current Core state. Accept an optional `ISS-` prefix so users
 * can type the number exactly as it is displayed elsewhere. */
export function filterBwRelationTargets(
  candidates: BwRelationTargetCandidate[],
  query: string,
): BwRelationTargetCandidate[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return candidates;
  const idQuery = normalized.replace(/^iss-/, "");
  return candidates.filter(
    (candidate) =>
      candidate.id.toLowerCase().includes(idQuery) ||
      candidate.reference.toLowerCase().includes(normalized) ||
      candidate.title.toLowerCase().includes(normalized) ||
      candidate.state.toLowerCase().includes(normalized),
  );
}

type BwTriageDelegation = {
  issue?: string;
  action?: string;
  delegate?: string;
  expires_at?: number;
};

/** True when the currently activated policy (`snapshot.activation.policy`,
 * whose own accepted body sits in `snapshot.records` like any other BW
 * record) grants `signer` a still-valid single-task delegation for this
 * exact `(issue, action)` pair — mirrors Core's own delegation check
 * (`crates/buzz-core/src/bw/semantics.rs::roles`, `"triage-action"` branch)
 * so the UI only offers `delegate: true` when Core would actually honor it.
 * Core re-decides this independently at submit time and remains the sole
 * authority; a false positive here only costs a doomed submit, never a
 * granted permission. */
export function bwMatchingTriageDelegation(
  snapshot: BwSnapshot,
  issueId: string,
  action: string,
  signer: string | null | undefined,
  now: number = Math.floor(Date.now() / 1000),
): boolean {
  if (!snapshot.activation || !signer) return false;
  const policy = snapshot.records[snapshot.activation.policy];
  if (!policy) return false;
  let delegations: BwTriageDelegation[];
  try {
    delegations =
      (
        JSON.parse(policy.content) as {
          triage_delegations?: BwTriageDelegation[];
        }
      ).triage_delegations ?? [];
  } catch {
    return false;
  }
  return delegations.some(
    (d) =>
      d.issue === issueId &&
      d.action === action &&
      d.delegate === signer &&
      typeof d.expires_at === "number" &&
      now < d.expires_at,
  );
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
    // Display only: the selected writer (`bwAssignmentHead`, the existing
    // kind:1 chain) so the facepile and detail rail show the current
    // assignee without a separate BW-unaware read path. Writing an
    // assignment for a BW issue goes through `submitBwAssignment`
    // (`bwWrite.ts`) — never the legacy `issueAssignments.ts` mutations,
    // which do not consult Core at all.
    const assignmentHead = bwAssignmentHead(snapshot, id);
    byId.set(id, {
      ...issue,
      title: fields?.title ?? issue.title,
      content: fields?.description ?? issue.content,
      status,
      workflowStatus: null,
      currentReview: null,
      assignees: assignmentHead.writer ? [assignmentHead.writer] : [],
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

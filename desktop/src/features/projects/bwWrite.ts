// BW record writes for already-enrolled issues (P4D): text/acceptance-
// criteria edits and the four triage actions available from the fixed
// triage state (need-info, accept-to-backlog, duplicate, decline). Every
// write goes through the single generic `submit_project_bw_record` Tauri
// command; role, causality, policy-binding and reference validity are all
// decided by the Core `Consumer` on the Rust side, never predicted here.
// This module's own job is narrower: compute the correct `previous` pointer
// from the already-fetched snapshot so a stale write cannot silently
// overwrite a concurrent one, and surface a real fork as a visible conflict
// instead of guessing a winner.
import { invokeTauri } from "@/shared/api/tauri";
import {
  bwAssignmentHead,
  bwChainHead,
  type BwAssignmentOperation,
  type BwSnapshot,
} from "./bwProjection";

export class BwConflictError extends Error {
  constructor(
    message = "Concurrent edits are in conflict. Reload and resolve manually.",
  ) {
    super(message);
  }
}

export type BwIssueTextPatch = {
  title?: string;
  description?: string;
  acceptance_criteria?: string[];
  non_goals?: string[];
};

/** Submit a text/acceptance-criteria/non-goals patch, chaining off the exact
 * current head. Throws `BwConflictError` up front when the chain is already
 * forked, rather than submitting a doomed write. A second, truly concurrent
 * writer racing this one is still caught: the Rust side re-fetches history
 * immediately before signing, so a `previous` that has already been
 * superseded is refused by Core (`wrong-previous`) instead of overwriting. */
export async function submitBwIssueTextUpdate({
  repo,
  issueId,
  snapshot,
  patch,
}: {
  repo: string;
  issueId: string;
  snapshot: BwSnapshot;
  patch: BwIssueTextPatch;
}): Promise<{ eventId: string; projection: unknown }> {
  const { headId, conflict } = bwChainHead(snapshot, issueId, "issue-update");
  if (conflict) {
    throw new BwConflictError();
  }
  const tags: string[][] = [["issue", issueId]];
  if (headId) tags.push(["previous", headId]);
  return invokeTauri("submit_project_bw_record", {
    input: {
      repo,
      record: "issue-update",
      tags,
      content: { patch },
      delegate: false,
    },
  });
}

export type BwTriageAction = "accept" | "need-info" | "duplicate" | "decline";

export type BwTriageActionFields =
  | { action: "accept" }
  | { action: "need-info"; question: string; recipient: string }
  | { action: "duplicate"; target: string }
  | { action: "decline"; reason: string };

/** Submit a triage action (Rückfrage/Backlog/Duplikat/Ablehnung). Whether
 * the signer is actually allowed to perform this action — Owner, an
 * operator (need-info only) or a matching triage delegation — is decided
 * exclusively by Core; an unauthorized attempt is refused with Core's own
 * `bw:reject:role:unauthorized` regardless of what the UI offered. */
export async function submitBwTriageAction({
  repo,
  issueId,
  snapshot,
  fields,
  delegate = false,
}: {
  repo: string;
  issueId: string;
  snapshot: BwSnapshot;
  fields: BwTriageActionFields;
  delegate?: boolean;
}): Promise<{ eventId: string; projection: unknown }> {
  const { headId, conflict } = bwChainHead(snapshot, issueId, "triage-action");
  if (conflict) {
    throw new BwConflictError();
  }
  const tags: string[][] = [["issue", issueId]];
  // A delegated action is single-use and may only be the first triage
  // action on the issue (NIP-BW.md: "authorizes one action at the issue's
  // initial triage head only"); chaining after an existing head is refused
  // by Core regardless, but there is no reason to submit that doomed chain.
  if (headId && !delegate) tags.push(["previous", headId]);
  return invokeTauri("submit_project_bw_record", {
    input: {
      repo,
      record: "triage-action",
      tags,
      content: fields,
      delegate,
    },
  });
}

// P4E: assignment (the *existing* kind:1 wire — no new grammar), relation
// (parent/child/blocks/duplicate-of) and the ready/in-development/implemented
// handoff. Every function below computes only the `previous`/`prior` pointer
// from the already-fetched snapshot and refuses up front on a visible fork;
// the actual role/causality/evidence decision is exclusively Core's, via
// `submit_project_bw_assignment` / `submit_project_bw_record`
// (`desktop/src-tauri/src/commands/project_bw_assignment.rs`,
// `project_bw_write.rs`).

/** Select or release the sole BW delegate for an issue, chaining off the
 * exact current kind:1 assignment-chain head. Whether this signer may
 * perform the operation at all (Owner or the coordinator active at this
 * moment) is decided exclusively by Core; an unauthorized attempt is
 * refused with its own `bw:reject:role:unauthorized`, never simplified or
 * pre-filtered here. */
export async function submitBwAssignment({
  repo,
  issueId,
  snapshot,
  delegate,
  operation,
}: {
  repo: string;
  issueId: string;
  snapshot: BwSnapshot;
  delegate: string;
  operation: BwAssignmentOperation;
}): Promise<{ eventId: string; projection: unknown }> {
  const { headId, conflict } = bwAssignmentHead(snapshot, issueId);
  if (conflict) {
    throw new BwConflictError();
  }
  return invokeTauri("submit_project_bw_assignment", {
    input: {
      repo,
      issueId,
      delegate,
      operation,
      prior: headId,
    },
  });
}

/** Move an issue from `backlog` to `ready`: binds a stream, the exact
 * current assignment-chain head (or `null` for an unassigned ready reset)
 * and the exact current text head. Core alone decides whether the signer is
 * Owner/coordinator, whether the bound assignment is actually current, and
 * whether a prior `implemented` requires a rework verdict or a terminal
 * failed/aborted set before this reset is allowed
 * (`crates/buzz-core/src/bw/semantics.rs::causality`, `"ready"` branch). */
export async function submitBwReadyTransition({
  repo,
  issueId,
  snapshot,
  stream,
  reworkVerdictId = null,
  terminalSetId = null,
}: {
  repo: string;
  issueId: string;
  snapshot: BwSnapshot;
  stream: string;
  reworkVerdictId?: string | null;
  terminalSetId?: string | null;
}): Promise<{ eventId: string; projection: unknown }> {
  const previousId = snapshot.projection.issue_state_id[issueId] ?? null;
  const { headId: updateId, conflict: updateConflict } = bwChainHead(
    snapshot,
    issueId,
    "issue-update",
  );
  if (updateConflict) {
    throw new BwConflictError();
  }
  const { headId: assignmentId, conflict: assignmentConflict } =
    bwAssignmentHead(snapshot, issueId);
  if (assignmentConflict) {
    throw new BwConflictError();
  }
  const tags: string[][] = [["issue", issueId]];
  if (previousId) tags.push(["previous", previousId]);
  const content: Record<string, unknown> = {
    state: "ready",
    stream,
    assignment: assignmentId,
    update: updateId,
    rework: reworkVerdictId,
  };
  if (terminalSetId) content.terminal_set = terminalSetId;
  return invokeTauri("submit_project_bw_record", {
    input: {
      repo,
      record: "issue-state",
      tags,
      content,
      delegate: false,
    },
  });
}

/** Move an issue from `ready` to `in-development`. Reuses the exact
 * stream/assignment the current `ready` head already carries — NIP-BW.md's
 * "writer-binding" rule requires an exact match, so this never lets a stale
 * or hand-typed value drift from what was actually selected at `ready`.
 * Only the delegate bound by that exact assignment head may sign; Core
 * decides that, never this function. */
export async function submitBwInDevelopmentTransition({
  repo,
  issueId,
  snapshot,
}: {
  repo: string;
  issueId: string;
  snapshot: BwSnapshot;
}): Promise<{ eventId: string; projection: unknown }> {
  const previousId = snapshot.projection.issue_state_id[issueId] ?? null;
  const current = snapshot.projection.issue_state[issueId];
  if (!previousId || !current?.stream || !current.assignment) {
    throw new Error(
      "This issue has no valid ready state with a bound assignment yet.",
    );
  }
  const tags: string[][] = [
    ["issue", issueId],
    ["previous", previousId],
  ];
  return invokeTauri("submit_project_bw_record", {
    input: {
      repo,
      record: "issue-state",
      tags,
      content: {
        state: "in-development",
        stream: current.stream,
        assignment: current.assignment,
      },
      delegate: false,
    },
  });
}

/** Move an issue from `in-development` to `implemented`. `commit` and
 * `remote_readback` are deliberately omitted here: the Tauri command
 * resolves both from an externally observed Git read of the repository's
 * own stream at submit time and overwrites anything sent for them —
 * NIP-BW.md: "The signed claim alone proves no remote fact." This function
 * cannot fabricate that evidence and does not try to; `tests` remains a
 * human-authored summary, exactly as the contract allows. */
export async function submitBwImplementedTransition({
  repo,
  issueId,
  snapshot,
  tests,
}: {
  repo: string;
  issueId: string;
  snapshot: BwSnapshot;
  tests: string;
}): Promise<{ eventId: string; projection: unknown }> {
  const previousId = snapshot.projection.issue_state_id[issueId] ?? null;
  const current = snapshot.projection.issue_state[issueId];
  if (!previousId || !current?.stream || !current.assignment) {
    throw new Error("This issue is not in development yet.");
  }
  const normalizedTests = tests.trim();
  if (!normalizedTests) {
    throw new Error("A tests summary is required.");
  }
  const tags: string[][] = [
    ["issue", issueId],
    ["previous", previousId],
  ];
  return invokeTauri("submit_project_bw_record", {
    input: {
      repo,
      record: "issue-state",
      tags,
      content: {
        state: "implemented",
        stream: current.stream,
        assignment: current.assignment,
        tests: normalizedTests,
      },
      delegate: false,
    },
  });
}

/** Add or remove a relation edge between two enrolled issues in the same
 * repository. Cycle rejection, active-membership locking and role
 * (Owner/coordinator-only) are all Core's decision
 * (`crates/buzz-core/src/bw/semantics.rs::causality`, `"issue-relation"`
 * branch) — this only chains off the exact current head for this precise
 * `(issue, relation, target)` triple, which is its own independent causal
 * chain distinct from any other relation on the same issue. */
export async function submitBwRelation({
  repo,
  issueId,
  snapshot,
  relation,
  target,
  operation,
}: {
  repo: string;
  issueId: string;
  snapshot: BwSnapshot;
  relation: "blocks" | "related" | "duplicate-of" | "parent-of";
  target: string;
  operation: "add" | "remove";
}): Promise<{ eventId: string; projection: unknown }> {
  const records = Object.values(snapshot.records).filter((event) => {
    if (event.kind !== 46100) return false;
    if (event.tags.find((tag) => tag[0] === "record")?.[1] !== "issue-relation")
      return false;
    if (event.tags.find((tag) => tag[0] === "issue")?.[1] !== issueId)
      return false;
    try {
      const body = JSON.parse(event.content) as {
        relation?: string;
        target?: string;
      };
      return body.relation === relation && body.target === target;
    } catch {
      return false;
    }
  });
  const referenced = new Set(
    records
      .map((event) => event.tags.find((tag) => tag[0] === "previous")?.[1])
      .filter((value): value is string => Boolean(value)),
  );
  const heads = records.filter((event) => !referenced.has(event.id));
  if (heads.length > 1) {
    throw new BwConflictError();
  }
  const tags: string[][] = [["issue", issueId]];
  if (heads.length === 1) tags.push(["previous", heads[0].id]);
  return invokeTauri("submit_project_bw_record", {
    input: {
      repo,
      record: "issue-relation",
      tags,
      content: { relation, target, operation },
      delegate: false,
    },
  });
}

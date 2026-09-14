// BW writes for already-enrolled issues. Text, non-accept triage actions,
// and relations retain the generic compatibility commands. Multi-step issue
// lifecycle actions use narrow Tauri business commands backed by the shared
// Rust operations layer. Core remains the only role and causality authority.
// This module's own job is narrower: compute the correct `previous` pointer
// from the already-fetched snapshot so a stale write cannot silently
// overwrite a concurrent one, and surface a real fork as a visible conflict
// instead of guessing a winner.
import { invokeTauri } from "@/shared/api/tauri";
import { bwChainHead, type BwSnapshot } from "./bwProjection";

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
  if (fields.action === "accept") {
    return invokeTauri("accept_project_bw_issue", {
      input: { delegated: delegate, issueId, repo },
    });
  }
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

// Generic Tauri compatibility commands remain available for text, relation,
// and low-level callers. Lifecycle sequencing lives in the shared Rust layer.

type BwWriteResult = { eventId: string; projection: unknown };

/** Select a writer from the backlog/ready dropdown. Replacing an existing
 * writer is the exact causal `unassignment -> assignment` sequence NIP-BW
 * requires. When the issue is already ready, finish by publishing a new ready
 * head bound to the new assignment; otherwise Start development would still
 * carry the old assignment ID and Core would correctly refuse it. The shared
 * Rust operation reloads history after every partial step and resumes safely. */
export async function submitBwWriterSelection({
  repo,
  issueId,
  delegate,
}: {
  repo: string;
  issueId: string;
  delegate: string;
}): Promise<BwWriteResult> {
  return invokeTauri("assign_project_bw_writer", {
    input: { issueId, repo, writer: delegate },
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
  stream,
  reworkVerdictId = null,
  terminalSetId = null,
}: {
  repo: string;
  issueId: string;
  stream: string;
  reworkVerdictId?: string | null;
  terminalSetId?: string | null;
}): Promise<{ eventId: string; projection: unknown }> {
  return invokeTauri("move_project_bw_issue_to_ready", {
    input: {
      issueId,
      repo,
      reworkVerdictId,
      stream,
      terminalSetId,
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
}: {
  repo: string;
  issueId: string;
}): Promise<{ eventId: string; projection: unknown }> {
  return invokeTauri("start_project_bw_development", {
    input: { issueId, repo },
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

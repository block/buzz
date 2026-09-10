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
    repo,
    record: "issue-update",
    tags,
    content: { patch },
    delegate: false,
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
    repo,
    record: "triage-action",
    tags,
    content: fields,
    delegate,
  });
}

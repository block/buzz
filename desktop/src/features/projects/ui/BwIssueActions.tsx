import * as React from "react";
import { toast } from "sonner";

import type {
  ProjectIssue,
  Repository as Project,
} from "@/features/projects/hooks";
import { useRepoStateQuery } from "@/features/projects/hooks";
import { bwMatchingTriageDelegation } from "@/features/projects/bwProjection";
import {
  BwConflictError,
  submitBwAcceptToBacklog,
  submitBwImplementedTransition,
  submitBwInDevelopmentTransition,
  submitBwIssueTextUpdate,
  submitBwReadyTransition,
  submitBwRelation,
  submitBwTriageAction,
  type BwTriageAction,
  type BwTriageActionFields,
} from "@/features/projects/bwWrite";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import { invokeTauri } from "@/shared/api/tauri";
import { BwAssignmentSection } from "./BwAssignmentSection";
import {
  bwReadyStreamOptions,
  bwReadyUpdateUnavailable,
  errorMessage,
  useInvalidateProjectIssues,
} from "./bwIssueActionsShared";

const RELATION_TYPES = [
  "blocks",
  "related",
  "duplicate-of",
  "parent-of",
] as const;

/** A BW-shaped root that has not (yet) been successfully enrolled — either
 * never attempted or a prior attempt was refused (e.g. a pre-cutover root:
 * Core reports `bw:reject:policy:cutover`). Retrying re-checks with Core,
 * never bypasses it. */
function EnrollIntoBw({
  issue,
  project,
}: {
  issue: ProjectIssue;
  project: Project;
}) {
  const [pending, setPending] = React.useState(false);
  const invalidate = useInvalidateProjectIssues(project);

  const handleEnroll = async () => {
    if (pending) return;
    setPending(true);
    try {
      await invokeTauri("submit_project_bw_record", {
        input: {
          repo: project.repoAddress,
          record: "issue-state",
          tags: [["issue", issue.id]],
          content: { state: "triage" },
          delegate: false,
        },
      });
      toast.success("Issue enrolled into BW.");
      await invalidate();
    } catch (error) {
      toast.error(errorMessage(error, "Failed to enroll issue into BW."));
    } finally {
      setPending(false);
    }
  };

  return (
    <div className="space-y-1.5" data-testid="bw-not-enrolled">
      <p className="text-sm text-muted-foreground">
        Not yet a BW issue — its history is not tracked by BW workflow rules.
      </p>
      <button
        className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
        data-testid="bw-enroll-issue"
        disabled={pending}
        onClick={() => void handleEnroll()}
        type="button"
      >
        {pending ? "Enrolling…" : "Enroll into BW"}
      </button>
    </div>
  );
}

function fieldsFor(issue: ProjectIssue) {
  return issue.bw?.snapshot.projection.issue_fields[issue.id];
}

/** Title/description/acceptance-criteria/non-goals editing for an enrolled
 * issue still in triage or backlog (BW disallows text edits once
 * in-development/implemented). Always chains off the exact current head;
 * a technical fork refuses the submit up front instead of overwriting. */
function BwTextEditor({
  issue,
  project,
}: {
  issue: ProjectIssue;
  project: Project;
}) {
  const fields = fieldsFor(issue);
  const [title, setTitle] = React.useState(fields?.title ?? issue.title);
  const [description, setDescription] = React.useState(
    fields?.description ?? issue.content,
  );
  const [criteria, setCriteria] = React.useState(
    (fields?.acceptance_criteria ?? []).join("\n"),
  );
  const [nonGoals, setNonGoals] = React.useState(
    (fields?.non_goals ?? []).join("\n"),
  );
  const [pending, setPending] = React.useState(false);
  const invalidate = useInvalidateProjectIssues(project);

  // biome-ignore lint/correctness/useExhaustiveDependencies: re-seed only when the issue identity or its BW snapshot changes, not on every unrelated re-render
  React.useEffect(() => {
    const current = fieldsFor(issue);
    setTitle(current?.title ?? issue.title);
    setDescription(current?.description ?? issue.content);
    setCriteria((current?.acceptance_criteria ?? []).join("\n"));
    setNonGoals((current?.non_goals ?? []).join("\n"));
  }, [issue.id, issue.bw?.snapshot]);

  const handleSubmit = async () => {
    if (pending || !issue.bw) return;
    const acceptanceCriteria = criteria
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
    if (acceptanceCriteria.length === 0) {
      toast.error("At least one acceptance criterion is required.");
      return;
    }
    setPending(true);
    try {
      await submitBwIssueTextUpdate({
        issueId: issue.id,
        patch: {
          acceptance_criteria: acceptanceCriteria,
          description: description.trim(),
          non_goals: nonGoals
            .split("\n")
            .map((line) => line.trim())
            .filter(Boolean),
          title: title.trim(),
        },
        repo: project.repoAddress,
        snapshot: issue.bw.snapshot,
      });
      toast.success("Issue text updated.");
      await invalidate();
    } catch (error) {
      toast.error(
        error instanceof BwConflictError
          ? error.message
          : errorMessage(error, "Failed to update issue text."),
      );
    } finally {
      setPending(false);
    }
  };

  return (
    <div className="space-y-2" data-testid="bw-text-editor">
      <label
        className="block text-xs text-muted-foreground"
        htmlFor="bw-edit-title"
      >
        Title
      </label>
      <input
        className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
        data-testid="bw-edit-title"
        disabled={pending}
        id="bw-edit-title"
        onChange={(event) => setTitle(event.target.value)}
        value={title}
      />
      <label
        className="block text-xs text-muted-foreground"
        htmlFor="bw-edit-description"
      >
        Description
      </label>
      <textarea
        className="min-h-16 w-full rounded-md border border-border/60 bg-background p-2 text-xs text-foreground"
        data-testid="bw-edit-description"
        disabled={pending}
        id="bw-edit-description"
        onChange={(event) => setDescription(event.target.value)}
        value={description}
      />
      <label
        className="block text-xs text-muted-foreground"
        htmlFor="bw-edit-acceptance-criteria"
      >
        Acceptance criteria (one per line)
      </label>
      <textarea
        className="min-h-16 w-full rounded-md border border-border/60 bg-background p-2 text-xs text-foreground"
        data-testid="bw-edit-acceptance-criteria"
        disabled={pending}
        id="bw-edit-acceptance-criteria"
        onChange={(event) => setCriteria(event.target.value)}
        value={criteria}
      />
      <label
        className="block text-xs text-muted-foreground"
        htmlFor="bw-edit-non-goals"
      >
        Non-goals (optional, one per line)
      </label>
      <textarea
        className="min-h-12 w-full rounded-md border border-border/60 bg-background p-2 text-xs text-foreground"
        data-testid="bw-edit-non-goals"
        disabled={pending}
        id="bw-edit-non-goals"
        onChange={(event) => setNonGoals(event.target.value)}
        value={nonGoals}
      />
      <button
        className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
        data-testid="bw-text-editor-submit"
        disabled={pending || !criteria.trim()}
        onClick={() => void handleSubmit()}
        type="button"
      >
        {pending ? "Saving…" : "Save text update"}
      </button>
    </div>
  );
}

/** The four contract-defined triage actions available from the `triage`
 * state. Every button stays visible regardless of the viewer's role: an
 * unauthorized attempt is refused by Core (never by hiding the button),
 * and its refusal reason is shown here. */
function BwTriageActions({
  issue,
  project,
}: {
  issue: ProjectIssue;
  project: Project;
}) {
  const [pending, setPending] = React.useState<BwTriageAction | null>(null);
  const [question, setQuestion] = React.useState("");
  const [recipient, setRecipient] = React.useState("");
  const [duplicateTarget, setDuplicateTarget] = React.useState("");
  const [declineReason, setDeclineReason] = React.useState("");
  const invalidate = useInvalidateProjectIssues(project);
  const identityQuery = useIdentityQuery();

  const run = async (action: BwTriageAction, fields: BwTriageActionFields) => {
    if (pending || !issue.bw) return;
    setPending(action);
    try {
      const delegate = bwMatchingTriageDelegation(
        issue.bw.snapshot,
        issue.id,
        action,
        identityQuery.data?.pubkey,
      );
      const accepted = await submitBwTriageAction({
        delegate,
        fields,
        issueId: issue.id,
        repo: project.repoAddress,
        snapshot: issue.bw.snapshot,
      });
      if (action === "accept") {
        // Core's own display projection already reports "backlog" once this
        // triage-action head is "accept" (`crates/buzz-core/src/bw/projection.rs`),
        // but the issue-state chain itself only advances once this separate,
        // Owner/coordinator-signed record lands (NIP-BW.md: "the accepting
        // triage action alone does not grant the triage delegate authority
        // to sign a state record") — otherwise `ready` stays refused with
        // `bw:reject:causality:state-transition`. Chained here so accept is
        // one action for the common owner/coordinator case; an unauthorized
        // delegate's attempt surfaces Core's own role refusal below exactly
        // like any other refusal in this panel, never a second gate.
        await submitBwAcceptToBacklog({
          issueId: issue.id,
          repo: project.repoAddress,
          snapshot: issue.bw.snapshot,
          triageId: accepted.eventId,
        });
      }
      toast.success("Triage action recorded.");
      setQuestion("");
      setRecipient("");
      setDuplicateTarget("");
      setDeclineReason("");
    } catch (error) {
      toast.error(
        error instanceof BwConflictError
          ? error.message
          : errorMessage(error, "Triage action was refused."),
      );
    } finally {
      setPending(null);
      await invalidate();
    }
  };

  const normalizedTarget = duplicateTarget.trim().toLowerCase();
  const targetIsKnownRoot =
    issue.bw?.snapshot.records[normalizedTarget]?.kind === 1621;
  const hexPubkey = /^[0-9a-f]{64}$/i.test(recipient.trim());

  return (
    <div className="space-y-3" data-testid="bw-triage-actions">
      <button
        className="rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground disabled:opacity-60"
        data-testid="bw-triage-accept"
        disabled={pending !== null}
        onClick={() => void run("accept", { action: "accept" })}
        type="button"
      >
        {pending === "accept" ? "Accepting…" : "Accept to backlog"}
      </button>

      <div className="space-y-1">
        <label
          className="block text-xs text-muted-foreground"
          htmlFor="bw-triage-question"
        >
          Rückfrage / clarification question
        </label>
        <input
          className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
          data-testid="bw-triage-question"
          id="bw-triage-question"
          onChange={(event) => setQuestion(event.target.value)}
          value={question}
        />
        <input
          className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
          data-testid="bw-triage-recipient"
          onChange={(event) => setRecipient(event.target.value)}
          placeholder="Recipient pubkey"
          value={recipient}
        />
        <button
          className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
          data-testid="bw-triage-need-info"
          disabled={pending !== null || !question.trim() || !hexPubkey}
          onClick={() =>
            void run("need-info", {
              action: "need-info",
              question: question.trim(),
              recipient: recipient.trim().toLowerCase(),
            })
          }
          type="button"
        >
          {pending === "need-info" ? "Sending…" : "Ask for clarification"}
        </button>
      </div>

      <div className="space-y-1">
        <label
          className="block text-xs text-muted-foreground"
          htmlFor="bw-triage-duplicate-target"
        >
          Duplicate of (issue id)
        </label>
        <input
          className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
          data-testid="bw-triage-duplicate-target"
          id="bw-triage-duplicate-target"
          onChange={(event) => setDuplicateTarget(event.target.value)}
          value={duplicateTarget}
        />
        {duplicateTarget.trim() && !targetIsKnownRoot ? (
          <p className="text-xs text-destructive">
            Unknown issue id in this repository.
          </p>
        ) : null}
        <button
          className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
          data-testid="bw-triage-duplicate"
          disabled={pending !== null || !targetIsKnownRoot}
          onClick={() =>
            void run("duplicate", {
              action: "duplicate",
              target: normalizedTarget,
            })
          }
          type="button"
        >
          {pending === "duplicate" ? "Marking…" : "Mark as duplicate"}
        </button>
      </div>

      <div className="space-y-1">
        <label
          className="block text-xs text-muted-foreground"
          htmlFor="bw-triage-decline-reason"
        >
          Decline reason
        </label>
        <textarea
          className="min-h-12 w-full rounded-md border border-border/60 bg-background p-2 text-xs text-foreground"
          data-testid="bw-triage-decline-reason"
          id="bw-triage-decline-reason"
          onChange={(event) => setDeclineReason(event.target.value)}
          value={declineReason}
        />
        <button
          className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
          data-testid="bw-triage-decline"
          disabled={pending !== null || !declineReason.trim()}
          onClick={() =>
            void run("decline", {
              action: "decline",
              reason: declineReason.trim(),
            })
          }
          type="button"
        >
          {pending === "decline" ? "Declining…" : "Decline"}
        </button>
      </div>
    </div>
  );
}

/** `backlog` -> `ready`: bind a stream and the exact current assignment
 * head. Core alone decides whether the signer is Owner/coordinator and
 * whether a prior implemented needs a rework verdict or terminal set first;
 * this only offers those two optional pointers as plain event-id inputs.
 *
 * The stream field is driven by the issue repository's actual branches
 * (`useRepoStateQuery`, the repo's owner-signed kind:30618 state — the same
 * remote-branch source `git ls-remote` would report), not a fixed platform-key
 * list or free text (Jari, 2026-09-12 scope amendment to the original issue).
 * A repo with more than one branch offers a dropdown; a repo with exactly one
 * branch pre-fills it as a read-only field — never an open dropdown or an
 * editable input for the single-option case (Robi, 2026-09-12 follow-up
 * correction). If the branch list can't be loaded, submit is blocked
 * outright — never a free-text escape.
 *
 * `update` is likewise blocked outright, not sent as a guessed/empty value,
 * when the issue has no `issue-update` head yet — see
 * `bwReadyUpdateUnavailable` in `bwIssueActionsShared.ts`. */
function BwReadyAction({
  issue,
  project,
}: {
  issue: ProjectIssue;
  project: Project;
}) {
  const committedStream =
    issue.bw?.snapshot.projection.issue_state[issue.id]?.stream ?? "";
  const [stream, setStream] = React.useState(committedStream);
  const [reworkVerdictId, setReworkVerdictId] = React.useState("");
  const [terminalSetId, setTerminalSetId] = React.useState("");
  const [pending, setPending] = React.useState(false);
  const invalidate = useInvalidateProjectIssues(project);
  const repoStateQuery = useRepoStateQuery(project);
  const { options: streamOptions, unavailable: streamUnavailable } =
    bwReadyStreamOptions(
      repoStateQuery.data?.branches,
      repoStateQuery.isLoading,
    );
  const updateUnavailable = issue.bw
    ? bwReadyUpdateUnavailable(issue.bw.snapshot, issue.id)
    : true;

  React.useEffect(() => {
    setStream(
      issue.bw?.snapshot.projection.issue_state[issue.id]?.stream ?? "",
    );
  }, [issue.id, issue.bw?.snapshot]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: re-pick only when the loaded option set (or the current selection's membership in it) actually changes, not on every unrelated re-render
  React.useEffect(() => {
    if (stream && streamOptions.includes(stream)) return;
    setStream(streamOptions[0] ?? "");
  }, [streamOptions]);

  const handleSubmit = async () => {
    if (pending || !issue.bw || !stream || updateUnavailable) return;
    setPending(true);
    try {
      await submitBwReadyTransition({
        issueId: issue.id,
        repo: project.repoAddress,
        reworkVerdictId: reworkVerdictId.trim() || null,
        snapshot: issue.bw.snapshot,
        stream,
        terminalSetId: terminalSetId.trim() || null,
      });
      toast.success("Issue moved to ready.");
      await invalidate();
    } catch (error) {
      toast.error(
        error instanceof BwConflictError
          ? error.message
          : errorMessage(error, "Ready transition was refused."),
      );
    } finally {
      setPending(false);
    }
  };

  return (
    <div className="space-y-1.5" data-testid="bw-ready-action">
      {streamOptions.length === 1 ? (
        <input
          className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground disabled:opacity-60"
          data-testid="bw-ready-stream"
          readOnly
          value={streamOptions[0]}
        />
      ) : (
        <select
          className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground disabled:opacity-60"
          data-testid="bw-ready-stream"
          disabled={repoStateQuery.isLoading || streamUnavailable}
          onChange={(event) => setStream(event.target.value)}
          value={stream}
        >
          {streamOptions.length === 0 ? (
            <option value="">
              {repoStateQuery.isLoading
                ? "Loading branches…"
                : "No branches available"}
            </option>
          ) : null}
          {streamOptions.map((branch) => (
            <option key={branch} value={branch}>
              {branch}
            </option>
          ))}
        </select>
      )}
      {streamUnavailable ? (
        <p
          className="text-xs text-destructive"
          data-testid="bw-ready-stream-unavailable"
          role="alert"
        >
          Branch list unavailable — cannot move to ready without a valid stream.
        </p>
      ) : null}
      {updateUnavailable ? (
        <p
          className="text-xs text-destructive"
          data-testid="bw-ready-update-unavailable"
          role="alert"
        >
          No saved text update yet — save a title/description/acceptance
          criteria update above before moving to ready.
        </p>
      ) : null}
      <input
        className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
        data-testid="bw-ready-rework-verdict"
        onChange={(event) => setReworkVerdictId(event.target.value)}
        placeholder="Rework verdict id (optional)"
        value={reworkVerdictId}
      />
      <input
        className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
        data-testid="bw-ready-terminal-set"
        onChange={(event) => setTerminalSetId(event.target.value)}
        placeholder="Terminal release-set id (optional)"
        value={terminalSetId}
      />
      <button
        className="rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground disabled:opacity-60"
        data-testid="bw-ready-submit"
        disabled={pending || !stream || updateUnavailable}
        onClick={() => void handleSubmit()}
        type="button"
      >
        {pending ? "Moving…" : "Move to ready"}
      </button>
    </div>
  );
}

/** `ready` -> `in-development`. Reuses the exact stream/assignment the
 * current `ready` head already carries, so this never needs its own
 * inputs. */
function BwInDevelopmentAction({
  issue,
  project,
}: {
  issue: ProjectIssue;
  project: Project;
}) {
  const [pending, setPending] = React.useState(false);
  const invalidate = useInvalidateProjectIssues(project);

  const handleSubmit = async () => {
    if (pending || !issue.bw) return;
    setPending(true);
    try {
      await submitBwInDevelopmentTransition({
        issueId: issue.id,
        repo: project.repoAddress,
        snapshot: issue.bw.snapshot,
      });
      toast.success("Issue moved to in-development.");
      await invalidate();
    } catch (error) {
      toast.error(
        error instanceof BwConflictError
          ? error.message
          : errorMessage(error, "In-development transition was refused."),
      );
    } finally {
      setPending(false);
    }
  };

  return (
    <button
      className="rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground disabled:opacity-60"
      data-testid="bw-in-development-submit"
      disabled={pending}
      onClick={() => void handleSubmit()}
      type="button"
    >
      {pending ? "Moving…" : "Start development"}
    </button>
  );
}

/** `in-development` -> `implemented`. `commit`/`remote_readback` are never
 * collected here — the Tauri command resolves both from an externally
 * observed Git read at submit time (NIP-BW.md: "The signed claim alone
 * proves no remote fact"). Only a human-authored tests summary is taken. */
function BwImplementedAction({
  issue,
  project,
}: {
  issue: ProjectIssue;
  project: Project;
}) {
  const [tests, setTests] = React.useState("");
  const [pending, setPending] = React.useState(false);
  const invalidate = useInvalidateProjectIssues(project);

  const handleSubmit = async () => {
    if (pending || !issue.bw || !tests.trim()) return;
    setPending(true);
    try {
      await submitBwImplementedTransition({
        issueId: issue.id,
        repo: project.repoAddress,
        snapshot: issue.bw.snapshot,
        tests: tests.trim(),
      });
      toast.success("Issue marked implemented.");
      await invalidate();
    } catch (error) {
      toast.error(
        error instanceof BwConflictError
          ? error.message
          : errorMessage(error, "Implemented transition was refused."),
      );
    } finally {
      setPending(false);
    }
  };

  return (
    <div className="space-y-1.5" data-testid="bw-implemented-action">
      <textarea
        className="min-h-16 w-full rounded-md border border-border/60 bg-background p-2 text-xs text-foreground"
        data-testid="bw-implemented-tests"
        onChange={(event) => setTests(event.target.value)}
        placeholder="Tests summary"
        value={tests}
      />
      <button
        className="rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground disabled:opacity-60"
        data-testid="bw-implemented-submit"
        disabled={pending || !tests.trim()}
        onClick={() => void handleSubmit()}
        type="button"
      >
        {pending ? "Submitting…" : "Mark implemented"}
      </button>
    </div>
  );
}

/** Read-only display of the `implemented` transition's own accepted
 * content — commit, tests summary and the externally observed readback
 * that proved it — literal passthrough of `issue_state`, never re-derived
 * or re-verified here. */
function BwImplementedDetails({ issue }: { issue: ProjectIssue }) {
  const fields = issue.bw?.snapshot.projection.issue_state[issue.id];
  if (!fields?.commit) return null;
  return (
    <dl
      className="space-y-1 text-xs text-muted-foreground"
      data-testid="bw-implemented-details"
    >
      <div>
        <dt className="inline font-medium text-foreground">Commit: </dt>
        <dd className="inline font-mono">{fields.commit}</dd>
      </div>
      {fields.tests ? (
        <div>
          <dt className="font-medium text-foreground">Tests</dt>
          <dd className="whitespace-pre-wrap">{fields.tests}</dd>
        </div>
      ) : null}
      {fields.remote_readback ? (
        <div>
          <dt className="inline font-medium text-foreground">
            Verified readback:{" "}
          </dt>
          <dd className="inline font-mono">
            {fields.remote_readback.stream}@{fields.remote_readback.head}
          </dd>
        </div>
      ) : null}
    </dl>
  );
}

/** Parent/child/blocks/duplicate-of relations for this issue, plus its
 * executable-leaf eligibility — both read verbatim from Core's projection
 * (`is_executable_leaf`, `relations`), never re-derived or simplified here
 * (cycles and closed-target checks are exclusively Core's decision on
 * submit). Only the four directly-owned relation kinds this issue is the
 * `issue` side of offer a Remove button; the displayed inverse edges
 * (child-of/blocked-by/duplicates) are read-only from here. */
function BwRelations({
  issue,
  project,
}: {
  issue: ProjectIssue;
  project: Project;
}) {
  const [relation, setRelation] =
    React.useState<(typeof RELATION_TYPES)[number]>("blocks");
  const [target, setTarget] = React.useState("");
  const [pending, setPending] = React.useState(false);
  const invalidate = useInvalidateProjectIssues(project);
  if (!issue.bw) return null;

  const relations = issue.bw.snapshot.projection.relations.filter(
    (r) => r.issue === issue.id,
  );
  const leaf = issue.bw.snapshot.projection.leaf[issue.id];
  const normalizedTarget = target.trim().toLowerCase();
  const targetIsKnownRoot =
    issue.bw.snapshot.records[normalizedTarget]?.kind === 1621;

  const run = async (op: "add" | "remove", rel: string, tgt: string) => {
    if (pending || !issue.bw) return;
    setPending(true);
    try {
      await submitBwRelation({
        issueId: issue.id,
        operation: op,
        relation: rel as (typeof RELATION_TYPES)[number],
        repo: project.repoAddress,
        snapshot: issue.bw.snapshot,
        target: tgt,
      });
      toast.success(op === "add" ? "Relation added." : "Relation removed.");
      setTarget("");
      await invalidate();
    } catch (error) {
      toast.error(
        error instanceof BwConflictError
          ? error.message
          : errorMessage(error, "Relation change was refused."),
      );
    } finally {
      setPending(false);
    }
  };

  return (
    <div className="space-y-2" data-testid="bw-relations">
      <p className="text-xs text-muted-foreground" data-testid="bw-leaf-state">
        {leaf
          ? "Executable leaf: eligible for a release freeze."
          : "Not an executable leaf (open child, duplicate or unresolved blocker)."}
      </p>
      <ul className="space-y-1">
        {relations.map((r) => (
          <li
            className="flex items-center justify-between gap-2 text-xs"
            data-testid="bw-relation-row"
            key={`${r.relation}:${r.target}`}
          >
            <span>
              {r.relation} <span className="font-mono">{r.target}</span>
            </span>
            {(RELATION_TYPES as readonly string[]).includes(r.relation) ? (
              <button
                className="rounded-md border border-border/60 px-2 py-0.5 text-xs disabled:opacity-60"
                data-testid="bw-relation-remove"
                disabled={pending}
                onClick={() => void run("remove", r.relation, r.target)}
                type="button"
              >
                Remove
              </button>
            ) : null}
          </li>
        ))}
      </ul>
      <div className="flex items-center gap-2">
        <select
          className="h-8 rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
          data-testid="bw-relation-type"
          onChange={(event) =>
            setRelation(event.target.value as (typeof RELATION_TYPES)[number])
          }
          value={relation}
        >
          {RELATION_TYPES.map((type) => (
            <option key={type} value={type}>
              {type}
            </option>
          ))}
        </select>
        <input
          className="h-8 flex-1 rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
          data-testid="bw-relation-target"
          onChange={(event) => setTarget(event.target.value)}
          placeholder="Target issue id"
          value={target}
        />
        <button
          className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
          data-testid="bw-relation-add"
          disabled={pending || !targetIsKnownRoot}
          onClick={() => void run("add", relation, normalizedTarget)}
          type="button"
        >
          Add
        </button>
      </div>
    </div>
  );
}

/** BW write surface for one issue: enroll retry for a not-yet-enrolled BW
 * root, text/acceptance-criteria editing while triage/backlog, the fixed
 * triage actions while in triage, writer assignment, the ready/
 * in-development/implemented handoff, and parent/child/blocks/duplicate-of
 * relations. */
export function BwIssueActions({
  issue,
  profiles,
  project,
}: {
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
}) {
  if (!issue.bw) return null;

  if (!issue.bw.enrolled) {
    if (issue.bw.notices.length > 0) return null;
    return <EnrollIntoBw issue={issue} project={project} />;
  }

  return (
    <div className="space-y-4">
      {issue.bw.fieldConflict ? (
        <p
          className="rounded-md border border-destructive/40 bg-destructive/10 p-2 text-xs text-destructive"
          data-testid="bw-field-conflict"
          role="alert"
        >
          Concurrent text edits are in conflict. Displayed fields are withheld
          until the conflict is resolved manually.
        </p>
      ) : null}
      {issue.bw.state === "triage" || issue.bw.state === "backlog" ? (
        <BwTextEditor issue={issue} project={project} />
      ) : null}
      {issue.bw.state === "triage" ? (
        <BwTriageActions issue={issue} project={project} />
      ) : null}
      {issue.bw.state === "backlog" || issue.bw.state === "ready" ? (
        <BwAssignmentSection
          issue={issue}
          profiles={profiles}
          project={project}
        />
      ) : null}
      {issue.bw.state === "backlog" ? (
        <BwReadyAction issue={issue} project={project} />
      ) : null}
      {issue.bw.state === "ready" ? (
        <BwInDevelopmentAction issue={issue} project={project} />
      ) : null}
      {issue.bw.state === "in-development" ? (
        <BwImplementedAction issue={issue} project={project} />
      ) : null}
      <BwImplementedDetails issue={issue} />
      <BwRelations issue={issue} project={project} />
    </div>
  );
}

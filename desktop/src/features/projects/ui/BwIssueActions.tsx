import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import type {
  ProjectIssue,
  Repository as Project,
} from "@/features/projects/hooks";
import {
  BwConflictError,
  submitBwIssueTextUpdate,
  submitBwTriageAction,
  type BwTriageAction,
  type BwTriageActionFields,
} from "@/features/projects/bwWrite";
import { invokeTauri } from "@/shared/api/tauri";

function useInvalidateProjectIssues(project: Project) {
  const queryClient = useQueryClient();
  return React.useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({
        queryKey: ["project", project.id, "issues"],
      }),
      queryClient.invalidateQueries({ queryKey: ["projects", "work-items"] }),
    ]);
  }, [project.id, queryClient]);
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

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
        repo: project.repoAddress,
        record: "issue-state",
        tags: [["issue", issue.id]],
        content: { state: "triage" },
        delegate: false,
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

  const run = async (action: BwTriageAction, fields: BwTriageActionFields) => {
    if (pending || !issue.bw) return;
    setPending(action);
    try {
      await submitBwTriageAction({
        fields,
        issueId: issue.id,
        repo: project.repoAddress,
        snapshot: issue.bw.snapshot,
      });
      toast.success("Triage action recorded.");
      setQuestion("");
      setRecipient("");
      setDuplicateTarget("");
      setDeclineReason("");
      await invalidate();
    } catch (error) {
      toast.error(
        error instanceof BwConflictError
          ? error.message
          : errorMessage(error, "Triage action was refused."),
      );
    } finally {
      setPending(null);
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

/** BW write surface for one issue: enroll retry for a not-yet-enrolled BW
 * root, text/acceptance-criteria editing while triage/backlog, and the
 * fixed triage actions while in triage. Renders nothing outside those
 * states (e.g. once in-development/implemented) — there is no writer
 * assignment or release UI here; that is a later phase. */
export function BwIssueActions({
  issue,
  project,
}: {
  issue: ProjectIssue;
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
    </div>
  );
}

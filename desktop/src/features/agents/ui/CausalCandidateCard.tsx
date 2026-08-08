import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Eye, GitBranch, ShieldCheck } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  appendCausalExperiment,
  readCausalLedger,
} from "@/shared/api/tauriCausalLedger";
import { sendChannelMessage } from "@/shared/api/tauri";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

import {
  buildApprovedReplayProposal,
  buildIndependentEvaluation,
  buildReplayDispatchMessage,
  buildReplayDispatchReceipt,
} from "../lib/causalReplayProposal";
import { inspectCausalCandidate } from "./causalCandidateInspection";

export function CausalCandidateCard({
  agentPubkey,
  channelId,
  sessionId,
}: {
  agentPubkey: string;
  channelId: string | null;
  sessionId: string | null;
}) {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ["causal-ledger", sessionId],
    queryFn: readCausalLedger,
    enabled: Boolean(sessionId),
    refetchInterval: 5_000,
  });
  const inspection = inspectCausalCandidate(query.data ?? [], sessionId);
  const experiments = query.data?.map((entry) => entry.experiment) ?? [];
  const candidate = experiments.find(
    (experiment) => experiment.experimentId === inspection?.experimentId,
  );
  const activeEvaluation = candidate?.evaluation ? candidate : undefined;
  const replayFromEvaluation = activeEvaluation
    ? experiments.find(
        (experiment) =>
          experiment.experimentId === activeEvaluation.execution.replayOf,
      )
    : undefined;
  const parentOfActive = candidate?.execution.replayOf
    ? experiments.find(
        (experiment) =>
          experiment.experimentId === candidate.execution.replayOf,
      )
    : undefined;
  const activeReplay =
    !candidate?.evaluation &&
    parentOfActive?.intervention.approvedAt &&
    !candidate?.execution.sessionId.startsWith("replay-dispatch:")
      ? candidate
      : undefined;
  const approvedProposal = replayFromEvaluation
    ? experiments.find(
        (experiment) =>
          experiment.experimentId === replayFromEvaluation.execution.replayOf,
      )
    : activeReplay
      ? parentOfActive
      : experiments.find(
          (experiment) =>
            experiment.execution.replayOf === inspection?.experimentId &&
            experiment.intervention.approvedAt,
        );
  const dispatchReceipt = experiments.find(
    (experiment) =>
      experiment.execution.replayOf === approvedProposal?.experimentId &&
      experiment.execution.sessionId.startsWith("replay-dispatch:"),
  );
  const completedReplay =
    replayFromEvaluation ??
    activeReplay ??
    experiments.find(
      (experiment) =>
        experiment.execution.replayOf === approvedProposal?.experimentId &&
        !experiment.execution.sessionId.startsWith("replay-dispatch:"),
    );
  const evaluation =
    activeEvaluation ??
    experiments.find(
      (experiment) =>
        experiment.execution.replayOf === completedReplay?.experimentId &&
        experiment.evaluation,
    );
  const [draft, setDraft] = React.useState({
    failureFingerprint: "",
    cause: "",
    changedVariable: "",
    successCriteria: "",
  });
  const approveMutation = useMutation({
    mutationFn: async () => {
      if (!candidate) throw new Error("The candidate is no longer available.");
      const recordedAt = new Date().toISOString();
      const proposal = buildApprovedReplayProposal(candidate, draft, {
        experimentId: `proposal:${candidate.experimentId}:${crypto.randomUUID()}`,
        recordedAt,
      });
      return appendCausalExperiment(proposal);
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["causal-ledger"] });
      toast.success("Controlled replay approved and sealed in the ledger.");
    },
    onError: (error) => {
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not approve the replay.",
      );
    },
  });
  const dispatchMutation = useMutation({
    mutationFn: async () => {
      if (!approvedProposal || !channelId) {
        throw new Error(
          "Open this candidate inside its channel to run the replay.",
        );
      }
      const result = await sendChannelMessage(
        channelId,
        buildReplayDispatchMessage(approvedProposal),
        undefined,
        undefined,
        [agentPubkey],
      );
      return appendCausalExperiment(
        buildReplayDispatchReceipt(
          approvedProposal,
          result.eventId,
          new Date().toISOString(),
        ),
      );
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["causal-ledger"] });
      toast.success("Controlled replay dispatched to a fresh agent turn.");
    },
    onError: (error) => {
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not dispatch the replay.",
      );
    },
  });
  if (!inspection) return null;

  return (
    <section
      aria-label="Causal ledger candidate"
      className="mb-3 overflow-hidden rounded-xl border border-border/70 bg-background"
      data-testid="causal-candidate-card"
    >
      <div className="flex flex-wrap items-start justify-between gap-2 border-b border-border/60 p-4">
        <div>
          <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
            Durable candidate
          </p>
          <h3 className="mt-1 text-sm font-semibold">{inspection.task}</h3>
        </div>
        <Badge variant="outline" className="capitalize">
          {inspection.status}
        </Badge>
      </div>

      <CandidateList
        icon={Eye}
        items={inspection.capturedFacts}
        label="Captured facts"
      />
      <CandidateList
        empty="No causal claim has been made from this raw session."
        icon={GitBranch}
        items={inspection.inferredClaims}
        label="Inferred claims"
      />
      <CandidateList
        empty="No known evidence gaps."
        icon={AlertTriangle}
        items={inspection.missingCoverage}
        label="Missing coverage"
        warning
      />

      <div className="bg-primary/[0.04] p-4">
        <div className="flex items-start gap-2">
          <ShieldCheck
            aria-hidden
            className="mt-0.5 h-4 w-4 shrink-0 text-primary"
          />
          <div>
            <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Promotion gate
            </p>
            <p className="mt-1 text-sm font-medium">{inspection.nextGate}</p>
          </div>
        </div>
      </div>

      {evaluation ? (
        <EvaluationResult evaluation={evaluation} />
      ) : completedReplay ? (
        <IndependentEvaluationForm
          replay={completedReplay}
          onSaved={async () => {
            await queryClient.invalidateQueries({
              queryKey: ["causal-ledger"],
            });
          }}
        />
      ) : approvedProposal ? (
        <div
          className="border-t border-border/60 p-4"
          data-testid="approved-replay-proposal"
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Owner-approved replay
            </p>
            <Badge variant="outline">
              {dispatchReceipt ? "Replay running" : "Ready to run"}
            </Badge>
          </div>
          <p className="mt-2 text-sm font-medium">
            Change only: {approvedProposal.intervention.changedVariable}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            Success: {approvedProposal.intervention.successCriteria}
          </p>
          <p className="mt-2 text-xs text-amber-700 dark:text-amber-300">
            Approval does not prove the remedy. Buzz will keep this untested
            until a linked replay and independent evidence produce a verdict.
          </p>
          {!dispatchReceipt ? (
            <Button
              className="mt-3"
              disabled={!channelId || dispatchMutation.isPending}
              onClick={() => dispatchMutation.mutate()}
              size="sm"
            >
              {dispatchMutation.isPending
                ? "Dispatching replay…"
                : "Run controlled replay"}
            </Button>
          ) : (
            <p className="mt-3 text-xs text-muted-foreground">
              Buzz linked the dispatch receipt. The independent verdict unlocks
              when the agent turn reaches a terminal event.
            </p>
          )}
        </div>
      ) : candidate?.result.outcome === "untested" ? (
        <ReplayApprovalForm
          draft={draft}
          isPending={approveMutation.isPending}
          onChange={(field, value) =>
            setDraft((current) => ({ ...current, [field]: value }))
          }
          onSubmit={() => approveMutation.mutate()}
        />
      ) : null}
    </section>
  );
}

function IndependentEvaluationForm({
  replay,
  onSaved,
}: {
  replay: import("../lib/causalLedger").CausalExperiment;
  onSaved: () => Promise<void>;
}) {
  const [outcome, setOutcome] = React.useState<
    "validated" | "rejected" | "inconclusive"
  >("inconclusive");
  const [evidenceIds, setEvidenceIds] = React.useState(
    replay.result.evidenceIds.join("\n"),
  );
  const [rationale, setRationale] = React.useState("");
  const mutation = useMutation({
    mutationFn: () => {
      const recordedAt = new Date().toISOString();
      return appendCausalExperiment(
        buildIndependentEvaluation(
          replay,
          { outcome, evidenceIds, rationale },
          {
            experimentId: `evaluation:${replay.experimentId}:${crypto.randomUUID()}`,
            recordedAt,
          },
        ),
      );
    },
    onSuccess: async () => {
      await onSaved();
      toast.success("Independent verdict sealed in the causal ledger.");
    },
    onError: (error) => {
      toast.error(
        error instanceof Error ? error.message : "Could not save the verdict.",
      );
    },
  });
  return (
    <form
      className="space-y-3 border-t border-border/60 p-4"
      data-testid="independent-evaluation-form"
      onSubmit={(event) => {
        event.preventDefault();
        mutation.mutate();
      }}
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          Independent evaluation
        </p>
        <Badge variant="outline">Replay complete</Badge>
      </div>
      <label className="block text-xs font-medium" htmlFor="causal-verdict">
        Verdict
      </label>
      <select
        className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
        id="causal-verdict"
        onChange={(event) => setOutcome(event.target.value as typeof outcome)}
        value={outcome}
      >
        <option value="validated">Validated</option>
        <option value="rejected">Rejected</option>
        <option value="inconclusive">Inconclusive</option>
      </select>
      <Textarea
        aria-label="Evaluation evidence IDs"
        onChange={(event) => setEvidenceIds(event.target.value)}
        placeholder="One evidence ID per line"
        value={evidenceIds}
      />
      <Textarea
        aria-label="Evaluation rationale"
        onChange={(event) => setRationale(event.target.value)}
        placeholder="Why the evidence meets or fails the success criteria"
        value={rationale}
      />
      <Button disabled={mutation.isPending} size="sm" type="submit">
        {mutation.isPending ? "Sealing verdict…" : "Seal independent verdict"}
      </Button>
    </form>
  );
}

function EvaluationResult({
  evaluation,
}: {
  evaluation: import("../lib/causalLedger").CausalExperiment;
}) {
  return (
    <div
      className="border-t border-border/60 p-4"
      data-testid="causal-evaluation-result"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          Independent verdict
        </p>
        <Badge variant="outline" className="capitalize">
          {evaluation.result.outcome}
        </Badge>
      </div>
      <p className="mt-2 text-sm">{evaluation.evaluation?.rationale}</p>
      <p className="mt-1 text-xs text-muted-foreground">
        {evaluation.result.evidenceIds.length} cited evidence receipt
        {evaluation.result.evidenceIds.length === 1 ? "" : "s"}; sealed in the
        owner-local ledger.
      </p>
    </div>
  );
}

function ReplayApprovalForm({
  draft,
  isPending,
  onChange,
  onSubmit,
}: {
  draft: {
    failureFingerprint: string;
    cause: string;
    changedVariable: string;
    successCriteria: string;
  };
  isPending: boolean;
  onChange: (field: keyof typeof draft, value: string) => void;
  onSubmit: () => void;
}) {
  return (
    <form
      className="space-y-3 border-t border-border/60 p-4"
      data-testid="replay-approval-form"
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit();
      }}
    >
      <div>
        <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          Controlled replay proposal
        </p>
        <p className="mt-1 text-xs text-muted-foreground">
          Name the cause, change exactly one variable, and define success before
          approving anything.
        </p>
      </div>
      <Input
        aria-label="Failure fingerprint"
        onChange={(event) => onChange("failureFingerprint", event.target.value)}
        placeholder="Failure fingerprint, for example host-write-bypass/v1"
        value={draft.failureFingerprint}
      />
      <Textarea
        aria-label="Cause hypothesis"
        onChange={(event) => onChange("cause", event.target.value)}
        placeholder="Evidence-backed cause hypothesis"
        value={draft.cause}
      />
      <Input
        aria-label="One changed variable"
        onChange={(event) => onChange("changedVariable", event.target.value)}
        placeholder="One changed variable"
        value={draft.changedVariable}
      />
      <Textarea
        aria-label="Success criteria"
        onChange={(event) => onChange("successCriteria", event.target.value)}
        placeholder="Observable success criteria"
        value={draft.successCriteria}
      />
      <Button disabled={isPending} size="sm" type="submit">
        {isPending ? "Sealing approval…" : "Approve controlled replay"}
      </Button>
    </form>
  );
}

function CandidateList({
  empty,
  icon: Icon,
  items,
  label,
  warning = false,
}: {
  empty?: string;
  icon: typeof Eye;
  items: readonly string[];
  label: string;
  warning?: boolean;
}) {
  return (
    <div className="border-b border-border/60 p-4">
      <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        {label}
      </p>
      {items.length ? (
        <ul className="mt-2 space-y-2">
          {items.map((item) => (
            <li
              className={
                warning
                  ? "flex gap-2 text-xs text-amber-700 dark:text-amber-300"
                  : "flex gap-2 text-xs"
              }
              key={item}
            >
              <Icon aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>{item}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-2 text-xs text-muted-foreground">{empty}</p>
      )}
    </div>
  );
}

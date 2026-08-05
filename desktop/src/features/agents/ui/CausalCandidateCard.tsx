import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Eye, GitBranch, ShieldCheck } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  appendCausalExperiment,
  readCausalLedger,
} from "@/shared/api/tauriCausalLedger";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

import { buildApprovedReplayProposal } from "../lib/causalReplayProposal";
import { inspectCausalCandidate } from "./causalCandidateInspection";

export function CausalCandidateCard({
  sessionId,
}: {
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
  const candidate = query.data
    ?.map((entry) => entry.experiment)
    .find((experiment) => experiment.experimentId === inspection?.experimentId);
  const approvedProposal = query.data
    ?.map((entry) => entry.experiment)
    .find(
      (experiment) =>
        experiment.execution.replayOf === inspection?.experimentId &&
        experiment.intervention.approvedAt,
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

      {approvedProposal ? (
        <div
          className="border-t border-border/60 p-4"
          data-testid="approved-replay-proposal"
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Owner-approved replay
            </p>
            <Badge variant="outline">Awaiting evaluator</Badge>
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

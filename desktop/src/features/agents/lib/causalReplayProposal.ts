import {
  CAUSAL_EXPERIMENT_SCHEMA,
  type CausalExperiment,
} from "./causalLedger";

export type ReplayProposalInput = {
  failureFingerprint: string;
  cause: string;
  changedVariable: string;
  successCriteria: string;
};

const REPLAY_MARKER_PREFIX = "buzz-controlled-replay:";

function required(label: string, value: string): string {
  const normalized = value.trim();
  if (!normalized) throw new Error(`${label} is required.`);
  return normalized;
}

export function buildApprovedReplayProposal(
  candidate: CausalExperiment,
  input: ReplayProposalInput,
  identity: { experimentId: string; recordedAt: string },
): CausalExperiment {
  if (candidate.result.outcome !== "untested") {
    throw new Error(
      "Only an untested candidate can start a controlled replay.",
    );
  }
  const failureFingerprint = required(
    "Failure fingerprint",
    input.failureFingerprint,
  );
  const cause = required("Cause hypothesis", input.cause);
  const changedVariable = required("Changed variable", input.changedVariable);
  const successCriteria = required("Success criteria", input.successCriteria);

  return {
    schema: CAUSAL_EXPERIMENT_SCHEMA,
    experimentId: identity.experimentId,
    recordedAt: identity.recordedAt,
    task: candidate.task,
    execution: {
      sessionId: `replay-proposal:${candidate.execution.sessionId}`,
      turnId: "pending-owner-approved-replay",
      replayOf: candidate.experimentId,
    },
    failureFingerprint,
    context: candidate.context,
    hypothesis: {
      cause,
      evidenceIds: [...candidate.result.evidenceIds],
    },
    intervention: {
      remedyId: `remedy:${identity.experimentId}`,
      changedVariable,
      successCriteria,
      approvedAt: identity.recordedAt,
    },
    result: { outcome: "untested", evidenceIds: [] },
    coverage: candidate.coverage,
    relations: {
      supports: [],
      contradicts: [],
      invalidates: [],
    },
  };
}

export function replayMarker(proposalId: string): string {
  return `[${REPLAY_MARKER_PREFIX}${proposalId}]`;
}

export function proposalIdFromReplayTask(description: string): string | null {
  const start = description.indexOf(`[${REPLAY_MARKER_PREFIX}`);
  if (start < 0) return null;
  const valueStart = start + REPLAY_MARKER_PREFIX.length + 1;
  const end = description.indexOf("]", valueStart);
  const value = end < 0 ? "" : description.slice(valueStart, end).trim();
  return value || null;
}

export function buildReplayDispatchMessage(proposal: CausalExperiment): string {
  if (
    !proposal.intervention.approvedAt ||
    !proposal.intervention.successCriteria
  ) {
    throw new Error("The replay must be approved before it can run.");
  }
  return [
    replayMarker(proposal.experimentId),
    "Run this as a controlled verification replay in a disposable workspace.",
    `Original task: ${proposal.task.description}`,
    `Change exactly one variable: ${proposal.intervention.changedVariable}`,
    `Keep fixed: code, model, tools, policy, and environment versions unless that named variable explicitly changes one of them.`,
    `Success criteria: ${proposal.intervention.successCriteria}`,
    "Report observable evidence and do not claim the remedy is validated; an independent owner evaluation happens after this turn.",
  ].join("\n\n");
}

export function buildReplayDispatchReceipt(
  proposal: CausalExperiment,
  eventId: string,
  recordedAt: string,
): CausalExperiment {
  if (!proposal.intervention.approvedAt) {
    throw new Error("The replay must be approved before it can be dispatched.");
  }
  return {
    ...proposal,
    experimentId: `dispatch:${proposal.experimentId}:${eventId}`,
    recordedAt,
    task: { ...proposal.task, sourceMessageId: eventId },
    execution: {
      sessionId: `replay-dispatch:${eventId}`,
      turnId: "awaiting-agent-session",
      replayOf: proposal.experimentId,
    },
    result: { outcome: "untested", evidenceIds: [`message:${eventId}`] },
    relations: { supports: [], contradicts: [], invalidates: [] },
  };
}

export function buildIndependentEvaluation(
  replay: CausalExperiment,
  input: {
    outcome: Exclude<CausalExperiment["result"]["outcome"], "untested">;
    evidenceIds: string;
    rationale: string;
  },
  identity: { experimentId: string; recordedAt: string },
): CausalExperiment {
  if (replay.result.outcome !== "untested" || !replay.execution.replayOf) {
    throw new Error("Only a completed, unevaluated replay can be evaluated.");
  }
  const evidenceIds = input.evidenceIds
    .split(/[\n,]/)
    .map((value) => value.trim())
    .filter(Boolean);
  if (!evidenceIds.length)
    throw new Error("At least one evidence ID is required.");
  if (
    input.outcome === "validated" &&
    Object.values(replay.coverage).some((coverage) => coverage !== "observed")
  ) {
    throw new Error(
      "A replay with missing coverage cannot be validated; choose inconclusive or rejected.",
    );
  }
  const rationale = required("Evaluation rationale", input.rationale);
  return {
    ...replay,
    experimentId: identity.experimentId,
    recordedAt: identity.recordedAt,
    execution: { ...replay.execution, replayOf: replay.experimentId },
    result: { outcome: input.outcome, evidenceIds },
    evaluation: {
      evaluator: "owner-independent",
      rationale,
      evaluatedAt: identity.recordedAt,
    },
    relations: {
      supports: input.outcome === "validated" ? [replay.experimentId] : [],
      contradicts: input.outcome === "rejected" ? [replay.experimentId] : [],
      invalidates: [],
    },
  };
}

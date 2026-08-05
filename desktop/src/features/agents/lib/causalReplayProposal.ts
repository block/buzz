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

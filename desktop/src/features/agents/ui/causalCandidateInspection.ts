import type {
  CausalExperiment,
  EvidenceCoverage,
  LedgerEntry,
} from "../lib/causalLedger";

export type CausalCandidateInspection = {
  experimentId: string;
  status: CausalExperiment["result"]["outcome"];
  task: string;
  capturedFacts: string[];
  inferredClaims: string[];
  missingCoverage: string[];
  nextGate: string;
};

const COVERAGE_LABELS: Record<string, string> = {
  acp_observer: "Agent Client Protocol activity",
  acp_permission_gate: "Agent Client Protocol permission decisions",
  host_workspace: "Host workspace effects",
  os_sandbox: "Operating-system sandbox effects",
};

function coverageLabel(key: string): string {
  return COVERAGE_LABELS[key] ?? key.replaceAll("_", " ");
}

function observedCoverage(
  coverage: Record<string, EvidenceCoverage>,
): string[] {
  return Object.entries(coverage)
    .filter(([, state]) => state === "observed")
    .map(([key]) => coverageLabel(key));
}

function missingCoverage(coverage: Record<string, EvidenceCoverage>): string[] {
  return Object.entries(coverage)
    .filter(([, state]) => state === "missing")
    .map(([key]) => coverageLabel(key));
}

export function inspectCausalCandidate(
  entries: readonly LedgerEntry[],
  sessionId: string | null,
): CausalCandidateInspection | null {
  if (!sessionId) return null;
  const experiment = [...entries]
    .reverse()
    .find(
      (entry) => entry.experiment.execution.sessionId === sessionId,
    )?.experiment;
  if (!experiment) return null;

  const capturedFacts = [
    `Session ${experiment.execution.sessionId} reached a terminal event.`,
    ...observedCoverage(experiment.coverage).map(
      (layer) => `${layer} evidence was captured.`,
    ),
    `${experiment.result.evidenceIds.length} evidence receipt${
      experiment.result.evidenceIds.length === 1 ? " was" : "s were"
    } attached.`,
  ];
  const inferredClaims =
    experiment.hypothesis.cause === "unclassified"
      ? []
      : [`Proposed cause: ${experiment.hypothesis.cause}`];
  const gaps = missingCoverage(experiment.coverage);
  const nextGate =
    experiment.result.outcome === "untested"
      ? "Classify the failure, approve one controlled change, then require an independent evaluator before promotion."
      : experiment.result.outcome === "inconclusive"
        ? "Collect the missing evidence and rerun the same controlled comparison."
        : "This result is evaluated; inspect its cited evidence before reusing the remedy.";

  return {
    experimentId: experiment.experimentId,
    status: experiment.result.outcome,
    task: experiment.task.description,
    capturedFacts,
    inferredClaims,
    missingCoverage: gaps,
    nextGate,
  };
}

export type EvalEvidenceKind = "outcome" | "process";

export interface EvalEvidence {
  id: string;
  kind: EvalEvidenceKind;
  source: string;
}

export interface EvalCriteria {
  id: string;
  version: number;
  ownerPubkey: string;
  lockedBeforeRun: boolean;
}

export interface EvalRunResult {
  agentPubkey: string;
  evaluatorPubkey: string;
  criteria: EvalCriteria;
  evidence: readonly EvalEvidence[];
  graderPassed: boolean | null;
}

export type EvalVerdictStatus = "validated" | "rejected" | "inconclusive";

export interface EvalVerdict {
  status: EvalVerdictStatus;
  missingCoverage: readonly string[];
  evidenceIds: readonly string[];
}

/**
 * Produces a conservative verdict from immutable criteria and independently
 * supplied outcome + process evidence. Missing trust inputs are inconclusive,
 * never silently converted into a failure or a pass.
 */
export function deriveEvalVerdict(run: EvalRunResult): EvalVerdict {
  const missingCoverage: string[] = [];
  const evidenceIds = new Set<string>();
  let hasOutcomeEvidence = false;
  let hasProcessEvidence = false;

  for (const evidence of run.evidence) {
    if (!evidence.id.trim() || !evidence.source.trim()) continue;
    evidenceIds.add(evidence.id);
    if (evidence.kind === "outcome") hasOutcomeEvidence = true;
    if (evidence.kind === "process") hasProcessEvidence = true;
  }

  if (!run.criteria.id.trim() || run.criteria.version < 1) {
    missingCoverage.push("versioned success criteria");
  }
  if (!run.criteria.ownerPubkey.trim()) {
    missingCoverage.push("criteria owner");
  }
  if (!run.criteria.lockedBeforeRun) {
    missingCoverage.push("criteria locked before run");
  }
  if (!run.evaluatorPubkey.trim() || run.evaluatorPubkey === run.agentPubkey) {
    missingCoverage.push("independent evaluator");
  }
  if (!hasOutcomeEvidence) missingCoverage.push("outcome evidence");
  if (!hasProcessEvidence) missingCoverage.push("process evidence");
  if (run.graderPassed === null) missingCoverage.push("grader verdict");

  if (missingCoverage.length > 0) {
    return {
      status: "inconclusive",
      missingCoverage,
      evidenceIds: [...evidenceIds],
    };
  }

  return {
    status: run.graderPassed ? "validated" : "rejected",
    missingCoverage,
    evidenceIds: [...evidenceIds],
  };
}

export interface EvalPromotionDecision {
  allowed: boolean;
  reasons: readonly string[];
}

/** Promotion additionally requires a non-regressing baseline comparison and owner review. */
export function deriveEvalPromotionDecision(input: {
  verdict: EvalVerdict;
  baselinePassRate: number | null;
  candidatePassRate: number | null;
  ownerApproved: boolean;
}): EvalPromotionDecision {
  const reasons: string[] = [];

  if (input.verdict.status !== "validated") {
    reasons.push("candidate verdict is not validated");
  }
  if (input.baselinePassRate === null || input.candidatePassRate === null) {
    reasons.push("baseline comparison is missing");
  } else if (input.candidatePassRate < input.baselinePassRate) {
    reasons.push("candidate regresses against baseline");
  }
  if (!input.ownerApproved) reasons.push("owner approval is missing");

  return { allowed: reasons.length === 0, reasons };
}

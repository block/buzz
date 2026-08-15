import type { CommandBrief } from "./briefContracts";

export type BriefDecision = Readonly<{
  key: string;
  runId: string;
  actionId: string;
  adviser: CommandBrief["contributions"][number]["adviser"];
  coaA: string;
  coaB?: string;
}>;

/** Projects validated proposals into the short decision surface. */
export function projectBriefDecisions(
  brief: Pick<CommandBrief, "runId" | "sections" | "contributions">,
): readonly BriefDecision[] {
  const admitted = new Set(
    brief.sections.decisions.map((finding) => finding.text),
  );
  const seen = new Set<string>();
  const decisions: BriefDecision[] = [];

  for (const contribution of brief.contributions) {
    for (const proposal of contribution.proposedActions) {
      if (!admitted.has(proposal.text) || seen.has(proposal.actionId)) continue;
      seen.add(proposal.actionId);
      decisions.push(
        Object.freeze({
          key: `${brief.runId}:${proposal.actionId}`,
          runId: brief.runId,
          actionId: proposal.actionId,
          adviser: contribution.adviser,
          coaA: proposal.text,
          ...(proposal.alternativeText
            ? { coaB: proposal.alternativeText }
            : {}),
        }),
      );
    }
  }

  return Object.freeze(decisions);
}

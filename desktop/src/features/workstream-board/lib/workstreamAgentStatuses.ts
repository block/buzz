import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import type { WorkstreamIdentity } from "@/features/workstream-board/lib/workstreamCardParser";
import type { WorkstreamWait } from "@/features/workstream-board/lib/workstreamWaits";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type WorkstreamAgentStatuses = {
  working: WorkstreamIdentity[];
  waiting: WorkstreamIdentity[];
};

/** Only projects states backed by current evidence. Non-active assignees remain unclassified. */
export function classifyWorkstreamAgents(
  assignees: readonly WorkstreamIdentity[],
  activeWorking: ActiveChannelTurnSummary | undefined,
  waits: readonly WorkstreamWait[],
): WorkstreamAgentStatuses {
  const active = new Set(
    activeWorking?.agentPubkeys.map((pubkey) => normalizePubkey(pubkey)) ?? [],
  );
  const waiting = new Set(
    waits.flatMap((wait) =>
      wait.actor.pubkey ? [normalizePubkey(wait.actor.pubkey)] : [],
    ),
  );
  const seen = new Set<string>();
  const unique = assignees.filter((assignee) => {
    const pubkey = normalizePubkey(assignee.pubkey);
    if (seen.has(pubkey)) return false;
    seen.add(pubkey);
    return true;
  });
  const working = unique.filter((assignee) =>
    active.has(normalizePubkey(assignee.pubkey)),
  );
  const workingKeys = new Set(
    working.map((assignee) => normalizePubkey(assignee.pubkey)),
  );
  return {
    working,
    waiting: unique.filter((assignee) => {
      const pubkey = normalizePubkey(assignee.pubkey);
      return !workingKeys.has(pubkey) && waiting.has(pubkey);
    }),
  };
}

import type { Channel } from "@/shared/api/types";
import { fromRawChannel, type RawChannel } from "@/shared/api/tauriChannels";
import { invokeTauri } from "@/shared/api/tauri";

type RawBestieAssignment = {
  agent_pubkey: string;
};

export type BestieAssignment = {
  agentPubkey: string;
};

export type BestieScope = {
  expectedRelayUrl: string;
  expectedSignerPubkey: string;
};

function fromRawAssignment(assignment: RawBestieAssignment): BestieAssignment {
  return {
    agentPubkey: assignment.agent_pubkey,
  };
}

export async function getBestieAssignment(
  scope: BestieScope,
): Promise<BestieAssignment | null> {
  const assignment = await invokeTauri<RawBestieAssignment | null>(
    "get_bestie_assignment",
    scope,
  );
  return assignment ? fromRawAssignment(assignment) : null;
}

export async function assignBestie(
  agentPubkey: string,
  scope: BestieScope,
): Promise<BestieAssignment> {
  return fromRawAssignment(
    await invokeTauri<RawBestieAssignment>("assign_bestie", {
      agentPubkey,
      ...scope,
    }),
  );
}

export async function clearBestieAssignment(scope: BestieScope): Promise<void> {
  await invokeTauri("clear_bestie_assignment", scope);
}

export async function resolveBestieConversation(
  scope: BestieScope,
): Promise<Channel> {
  return fromRawChannel(
    await invokeTauri<RawChannel>("resolve_bestie_conversation", scope),
  );
}

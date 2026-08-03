import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

type DmParticipant = {
  pubkey: string;
};

export function resolveDmCodexVoiceAgent(
  channelType: string | undefined,
  participants: DmParticipant[],
  managedAgents: ManagedAgent[],
): { name: string; pubkey: string } | null {
  if (channelType !== "dm" || participants.length !== 1) return null;
  const participantPubkey = normalizePubkey(participants[0].pubkey);
  const agent = managedAgents.find(
    (candidate) => normalizePubkey(candidate.pubkey) === participantPubkey,
  );
  if (!agent) return null;
  return { name: agent.name, pubkey: agent.pubkey };
}

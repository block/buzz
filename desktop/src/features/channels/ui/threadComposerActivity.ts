import { truncatePubkey } from "@/shared/lib/pubkey";

type ActivityAgent = { pubkey: string; name: string };
type ActivityProfile = { displayName?: string | null };

export function completeActivityAgentRoster(
  agents: readonly ActivityAgent[],
  workingPubkeys: readonly string[],
  profiles: Readonly<Record<string, ActivityProfile | undefined>>,
): ActivityAgent[] {
  const completed = [...agents];
  const known = new Set(agents.map((agent) => agent.pubkey.toLowerCase()));

  for (const pubkey of workingPubkeys) {
    const normalized = pubkey.toLowerCase();
    if (known.has(normalized)) continue;
    known.add(normalized);
    completed.push({
      pubkey,
      name: profiles[normalized]?.displayName || truncatePubkey(pubkey),
    });
  }

  return completed;
}

export function composeThreadActivityPubkeys(
  channelWorkingPubkeys: readonly string[],
  threadTypingPubkeys: readonly string[],
): string[] {
  const seen = new Set<string>();
  return [...channelWorkingPubkeys, ...threadTypingPubkeys].filter((pubkey) => {
    const normalized = pubkey.toLowerCase();
    if (seen.has(normalized)) return false;
    seen.add(normalized);
    return true;
  });
}

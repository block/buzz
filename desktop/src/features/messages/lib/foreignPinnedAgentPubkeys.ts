import { normalizePubkey } from "@/shared/lib/pubkey";

/** Scheme-defaulted, slash-trimmed, case-folded form for pin comparison. */
function canonicalRelayUrl(url: string): string {
  const trimmed = url.trim().replace(/\/+$/, "").toLowerCase();
  return trimmed.startsWith("ws://") || trimmed.startsWith("wss://")
    ? trimmed
    : `wss://${trimmed}`;
}

/**
 * Pubkeys of the user's own managed-agent instances that are pinned to a
 * different community's relay (#2515). A pinned instance runs — and answers
 * mentions — only on its own relay, so it must not be a mention candidate in
 * the active community, even when a stale channel membership still lists it.
 * Unpinned instances follow the active workspace relay and are never foreign.
 */
export function foreignPinnedAgentPubkeys(
  agents: readonly { pubkey: string; relayUrl?: string | null }[],
  activeRelayUrl: string | null | undefined,
): Set<string> {
  const foreign = new Set<string>();
  const activeRelay = activeRelayUrl?.trim();
  if (!activeRelay) return foreign;
  const active = canonicalRelayUrl(activeRelay);
  for (const agent of agents) {
    const pin = agent.relayUrl?.trim();
    if (pin && canonicalRelayUrl(pin) !== active) {
      foreign.add(normalizePubkey(agent.pubkey));
    }
  }
  return foreign;
}

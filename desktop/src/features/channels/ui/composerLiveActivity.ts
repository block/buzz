/**
 * Selection logic for the composer live-activity preview.
 *
 * The popover shows ONE working agent's feed at a time. Resolution order:
 * explicit tab selection, then the agent whose session pane is already open,
 * then the first working agent. A selection that stops working (its agent
 * leaves the list) silently falls through to the next candidate rather than
 * pinning a dead tab.
 */
export function resolveSelectedActivityAgent<T extends { pubkey: string }>({
  openAgentSessionPubkey,
  selectedPubkey,
  workingAgents,
}: {
  openAgentSessionPubkey: string | null;
  selectedPubkey: string | null;
  workingAgents: readonly T[];
}): T | null {
  const findByPubkey = (pubkey: string | null) =>
    pubkey
      ? (workingAgents.find(
          (agent) => agent.pubkey.toLowerCase() === pubkey.toLowerCase(),
        ) ?? null)
      : null;

  return (
    findByPubkey(selectedPubkey) ??
    findByPubkey(openAgentSessionPubkey) ??
    workingAgents[0] ??
    null
  );
}

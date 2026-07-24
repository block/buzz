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

/**
 * Latest activity timestamp (ms) for the composer preview's "Last live" pill.
 *
 * Reads the same sources the preview panel renders — the live transcript
 * window AND the channel-scoped archive — so the pill can never claim
 * "No activity yet" while archived rows are visible underneath. Falls back
 * to the active-turn anchor when a turn is running but no items exist yet.
 */
export function deriveLastLiveAt({
  activeTurns,
  archivedEvents,
  channelId,
  transcript,
}: {
  activeTurns: readonly { anchorAt: number; channelId: string }[];
  archivedEvents: readonly { timestamp: string }[];
  channelId: string | null;
  transcript: readonly {
    channelId?: string | null;
    timestamp: string;
  }[];
}): number | null {
  let latest: number | null = null;
  const record = (timestamp: number) => {
    if (latest === null || timestamp > latest) {
      latest = timestamp;
    }
  };

  for (let index = transcript.length - 1; index >= 0; index -= 1) {
    const item = transcript[index];
    if (!item || (channelId && item.channelId !== channelId)) {
      continue;
    }
    const millis = Date.parse(item.timestamp);
    if (!Number.isNaN(millis)) {
      record(millis);
      break;
    }
  }

  // Archived events are already channel-scoped by the store, sorted ascending.
  for (let index = archivedEvents.length - 1; index >= 0; index -= 1) {
    const event = archivedEvents[index];
    if (!event) {
      continue;
    }
    const millis = Date.parse(event.timestamp);
    if (!Number.isNaN(millis)) {
      record(millis);
      break;
    }
  }

  const channelTurn = channelId
    ? activeTurns.find((turn) => turn.channelId === channelId)
    : activeTurns[0];
  if (channelTurn) {
    record(channelTurn.anchorAt);
  }

  return latest;
}

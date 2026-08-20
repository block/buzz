import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import type { Channel } from "@/shared/api/types";

/**
 * Restricts the shared liveness projection to cards that exist on this board.
 * It intentionally does not inspect observer frames or time: the active-turn
 * store is the only owner of those semantics.
 */
export function getActiveWorkstreamTurns(
  channels: readonly Channel[],
  activeTurnsByChannel: readonly ActiveChannelTurnSummary[],
): ActiveChannelTurnSummary[] {
  const channelIds = new Set(channels.map((channel) => channel.id));
  return activeTurnsByChannel.filter((turn) => channelIds.has(turn.channelId));
}

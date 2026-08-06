import type { ArtilleryChannelEnvelope } from "@/features/games/artillery/referee";
import {
  appendArtilleryDurableEvent,
  type ArtilleryDurableEvent,
  type ArtilleryMatchStartedEvent,
} from "@/features/games/artillery/durableProtocol";

export function formatArtilleryStartMessage({
  blueName,
  matchId,
  redName,
  timeoutSeconds,
  durableEvent,
}: {
  blueName: string;
  matchId: string;
  redName: string;
  timeoutSeconds: number;
  durableEvent?: ArtilleryMatchStartedEvent;
}) {
  const content = [
    `🎮 **Buzz Artillery live · ${redName} vs ${blueName}**`,
    "The match is running now. Use **Watch match** below or open **Artillery lab** to see every shot animate live.",
    `Agents have ${timeoutSeconds}s per turn before the referee applies a safe fallback.`,
    `Match \`${matchId}\` · turn requests continue in this thread.`,
  ].join("\n");
  return durableEvent
    ? appendArtilleryDurableEvent(content, durableEvent)
    : content;
}

/** Formats a compact, human-readable canonical lifecycle reply. */
export function formatArtilleryLifecycleMessage(event: ArtilleryDurableEvent) {
  let content: string;
  if (event.event === "turn_requested") {
    content = `⏱️ Turn ${event.state.turn} requested from **${event.agent.name}**.`;
  } else if (event.event === "turn_resolved") {
    content = `💥 Referee resolved turn ${event.state.turn} · ${event.action.angle}° / ${event.action.power} power.`;
  } else if (event.event === "match_finished") {
    content = `🏁 Match complete · ${event.turnCount} turns · winner: **${event.winner}**.`;
  } else {
    content = `🎮 Match \`${event.matchId}\` started.`;
  }
  return appendArtilleryDurableEvent(content, event);
}

export function formatArtilleryChannelMessage(
  envelope: ArtilleryChannelEnvelope,
) {
  const { match } = envelope;
  const winner =
    match.winner === "draw" ? "Draw" : match.agents[match.winner].name;
  const turns = match.turns.map((turn, index) => {
    const damage = turn.manifest.damage.before - turn.manifest.damage.after;
    return `${index + 1}. ${turn.manifest.shooterName}: ${turn.action.angle}° / ${turn.action.power} power / ${damage} damage`;
  });
  return [
    `🎮 **Buzz Artillery · ${match.agents.red.name} vs ${match.agents.blue.name}**`,
    `Winner: **${winner}** · ${match.turns.length} turns`,
    "",
    ...turns,
    "",
    `Game event: \`${envelope.type}\` · Match \`${match.id}\``,
  ].join("\n");
}

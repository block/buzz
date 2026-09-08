import type { ObserverEvent } from "@/shared/runtime/observerEvent";
import type { ActivityItem, ActivityStatus, AgentTurn } from "./types";

const TERMINAL_KINDS: Record<string, ActivityStatus> = {
  turn_completed: "completed",
  turn_failed: "failed",
  turn_cancelled: "cancelled",
  permission_requested: "needs_you",
  turn_unavailable: "unavailable",
};

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function statusFor(event: ObserverEvent): ActivityStatus {
  if (TERMINAL_KINDS[event.kind]) return TERMINAL_KINDS[event.kind];
  const payloadStatus = text(event.payload.status);
  if (
    payloadStatus &&
    [
      "pending",
      "running",
      "completed",
      "failed",
      "cancelled",
      "needs_you",
      "unavailable",
    ].includes(payloadStatus)
  ) {
    return payloadStatus as ActivityStatus;
  }
  return event.kind.includes("completed") ? "completed" : "running";
}

function itemLabel(event: ObserverEvent): string {
  return (
    text(event.payload.title) ??
    text(event.payload.toolCallTitle) ??
    text(event.payload.toolName) ??
    text(event.payload.name) ??
    event.kind.replaceAll("_", " ")
  );
}

function itemId(event: ObserverEvent): string {
  return (
    text(event.payload.toolCallId) ??
    text(event.payload.messageId) ??
    `${event.kind}:${event.seq}`
  );
}

function eventToItem(event: ObserverEvent): ActivityItem {
  return {
    id: itemId(event),
    label: itemLabel(event),
    detail:
      text(event.payload.summary) ??
      text(event.payload.outcome) ??
      text(event.payload.error),
    status: statusFor(event),
    timestamp: event.timestamp,
  };
}

/**
 * Projects one owner-private observer event into a stable agent turn. The
 * capability owns this projection once; conversations only read its result.
 */
export function reduceActivity(
  current: Map<string, AgentTurn>,
  event: ObserverEvent,
  fallbackAgentName = "Agent",
): Map<string, AgentTurn> {
  if (!event.channelId || !event.agentPubkey || !event.turnId) return current;
  const sessionId = event.sessionId ?? "unknown-session";
  const key = [
    event.channelId,
    event.agentPubkey,
    sessionId,
    event.turnId,
  ].join(":");
  const next = new Map(current);
  const previous = next.get(key);
  const incoming = eventToItem(event);
  const items = previous ? [...previous.items] : [];
  const existingIndex = items.findIndex((item) => item.id === incoming.id);
  if (existingIndex >= 0) items[existingIndex] = incoming;
  else items.push(incoming);
  items.sort((left, right) => left.timestamp.localeCompare(right.timestamp));
  // A completed tool call is evidence within an active turn, not completion
  // of the turn itself. Only a terminal turn event may replace its aggregate
  // state; otherwise the dock would disappear midway through an agent's work.
  const isTerminalTurnEvent = event.kind in TERMINAL_KINDS;
  const turnStatus = isTerminalTurnEvent
    ? statusFor(event)
    : previous
      ? previous.status
      : statusFor(event);

  next.set(key, {
    key,
    agentPubkey: event.agentPubkey,
    agentName: text(event.payload.agentName) ?? fallbackAgentName,
    sessionId,
    turnId: event.turnId,
    channelId: event.channelId,
    status: turnStatus,
    items,
  });
  return next;
}

/** The most recent activity item is the compact dock's truthful summary. */
export function latestActivityItem(turn: AgentTurn): ActivityItem | undefined {
  return turn.items.at(-1);
}

export function partitionActivity(turn: AgentTurn): {
  visibleItems: ActivityItem[];
  hiddenItems: ActivityItem[];
} {
  const recentIds = new Set(turn.items.slice(-3).map((item) => item.id));
  const urgentIds = new Set(
    turn.items
      .filter((item) =>
        ["failed", "needs_you", "cancelled", "unavailable"].includes(
          item.status,
        ),
      )
      .map((item) => item.id),
  );
  return {
    visibleItems: turn.items.filter(
      (item) => recentIds.has(item.id) || urgentIds.has(item.id),
    ),
    hiddenItems: turn.items.filter(
      (item) => !recentIds.has(item.id) && !urgentIds.has(item.id),
    ),
  };
}

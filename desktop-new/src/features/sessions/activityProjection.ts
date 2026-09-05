import type {
  ActivityItem,
  ActivityStatus,
  AgentTurn,
  ObserverEvent,
} from "./types";

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
  const title =
    text(event.payload.title) ??
    text(event.payload.toolCallTitle) ??
    text(event.payload.toolName) ??
    text(event.payload.name);
  if (title) return title;
  return event.kind.replaceAll("_", " ");
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
  next.set(key, {
    key,
    agentPubkey: event.agentPubkey,
    agentName: text(event.payload.agentName) ?? fallbackAgentName,
    sessionId,
    turnId: event.turnId,
    status: statusFor(event),
    items,
  });
  return next;
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
  const visibleItems = turn.items.filter(
    (item) => recentIds.has(item.id) || urgentIds.has(item.id),
  );
  const visibleIds = new Set(visibleItems.map((item) => item.id));
  return {
    visibleItems,
    hiddenItems: turn.items.filter((item) => !visibleIds.has(item.id)),
  };
}

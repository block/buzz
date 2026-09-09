import * as React from "react";

import {
  type AgentObserverStoreUpdate,
  compareObserverEvents,
  getAgentObserverSnapshot,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import { friendlyTurnErrorCopy } from "@/features/agents/lib/friendlyAgentLastError";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ObserverEvent } from "./ui/agentSessionTypes";

const MAX_FAILURES_PER_AGENT = 20;
const MAX_TURN_CONTEXTS_PER_AGENT = 3_000;
const EMPTY_FAILURES: RecentAgentTurnFailure[] = [];

type TurnContext = {
  channelId: string;
  rootEventId: string | null;
  parentEventId: string | null;
  triggeringEventIds: string[];
};

export type TurnFailureDisposition =
  | "retrying"
  | "dead_lettered"
  | "action_required"
  | "respawning"
  | "stopped";

export type RecentAgentTurnFailure = TurnContext & {
  agentPubkey: string;
  turnId: string;
  error: string;
  disposition: TurnFailureDisposition;
  attempt: number | null;
  timestamp: string;
};

const failuresByAgent = new Map<string, Map<string, RecentAgentTurnFailure>>();
const turnContextsByAgent = new Map<string, Map<string, TurnContext>>();
const lastProcessedByAgent = new Map<string, Map<string, ObserverEvent>>();
const listeners = new Set<() => void>();
const cachedByScope = new Map<string, RecentAgentTurnFailure[]>();

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : {};
}

function asString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function eventIds(payload: Record<string, unknown>): string[] {
  return Array.isArray(payload.triggeringEventIds)
    ? payload.triggeringEventIds.filter(
        (id): id is string => typeof id === "string" && id.length > 0,
      )
    : [];
}

function contextFromEvent(event: ObserverEvent): TurnContext | null {
  if (!event.channelId) return null;
  const payload = asRecord(event.payload);
  const ids = eventIds(payload);
  const rootEventId = asString(payload.triggeringRootEventId);
  const parentEventId = asString(payload.triggeringParentEventId);
  if (ids.length === 0 && !rootEventId && !parentEventId) return null;
  return {
    channelId: event.channelId,
    rootEventId: rootEventId ?? ids[ids.length - 1] ?? null,
    parentEventId: parentEventId ?? ids[ids.length - 1] ?? null,
    triggeringEventIds: ids,
  };
}

function failureKey(context: TurnContext): string {
  return `${context.channelId}:${context.rootEventId ?? "unknown"}`;
}

function disposition(value: unknown): TurnFailureDisposition {
  switch (value) {
    case "retrying":
    case "dead_lettered":
    case "action_required":
    case "respawning":
    case "stopped":
      return value;
    default:
      return "stopped";
  }
}

function invalidate() {
  cachedByScope.clear();
}

function notify() {
  invalidate();
  for (const listener of listeners) listener();
}

function rememberTurnContext(
  agentKey: string,
  turnId: string,
  context: TurnContext,
) {
  let contexts = turnContextsByAgent.get(agentKey);
  if (!contexts) {
    contexts = new Map();
    turnContextsByAgent.set(agentKey, contexts);
  }
  contexts.delete(turnId);
  contexts.set(turnId, context);
  while (contexts.size > MAX_TURN_CONTEXTS_PER_AGENT) {
    const oldest = contexts.keys().next().value;
    if (oldest === undefined) break;
    contexts.delete(oldest);
  }
}

function removeFailure(agentKey: string, context: TurnContext): boolean {
  const failures = failuresByAgent.get(agentKey);
  if (!failures?.delete(failureKey(context))) return false;
  if (failures.size === 0) failuresByAgent.delete(agentKey);
  return true;
}

function setFailure(agentKey: string, failure: RecentAgentTurnFailure) {
  let failures = failuresByAgent.get(agentKey);
  if (!failures) {
    failures = new Map();
    failuresByAgent.set(agentKey, failures);
  }
  const key = failureKey(failure);
  failures.delete(key);
  failures.set(key, failure);
  while (failures.size > MAX_FAILURES_PER_AGENT) {
    const oldest = failures.keys().next().value;
    if (oldest === undefined) break;
    failures.delete(oldest);
  }
}

function processEvent(agentPubkey: string, event: ObserverEvent): boolean {
  const agentKey = normalizePubkey(agentPubkey);
  const channelKey = event.channelId ?? "\u0000null-channel";
  let watermarks = lastProcessedByAgent.get(agentKey);
  const prior = watermarks?.get(channelKey);
  if (prior && compareObserverEvents(event, prior) <= 0) return false;
  if (!watermarks) {
    watermarks = new Map();
    lastProcessedByAgent.set(agentKey, watermarks);
  }
  watermarks.set(channelKey, event);

  const turnId = event.turnId;
  if (event.kind === "turn_started" && turnId) {
    const context = contextFromEvent(event);
    if (!context) return false;
    rememberTurnContext(agentKey, turnId, context);
    return removeFailure(agentKey, context);
  }

  if (event.kind !== "turn_error" && event.kind !== "agent_panic") {
    return false;
  }

  const payload = asRecord(event.payload);
  const context =
    contextFromEvent(event) ??
    (turnId ? turnContextsByAgent.get(agentKey)?.get(turnId) : null);
  if (!context || !turnId) return false;
  const rawError = asString(payload.error) ?? "Unknown error";
  const numericAttempt = Number(payload.attempt);
  setFailure(agentKey, {
    ...context,
    agentPubkey,
    turnId,
    error: friendlyTurnErrorCopy(rawError, payload.code),
    disposition: disposition(payload.disposition),
    attempt:
      Number.isInteger(numericAttempt) && numericAttempt > 0
        ? numericAttempt
        : null,
    timestamp: event.timestamp,
  });
  return true;
}

export function syncRecentAgentTurnFailuresFromEvents(
  agentPubkey: string,
  events: ObserverEvent[],
) {
  let changed = false;
  for (const event of events) {
    changed = processEvent(agentPubkey, event) || changed;
  }
  if (changed) notify();
}

export function syncRecentAgentTurnFailuresFromObserver(
  agents: readonly { pubkey: string; status: string }[],
) {
  for (const agent of agents) {
    if (agent.status !== "running" && agent.status !== "deployed") continue;
    syncRecentAgentTurnFailuresFromEvents(
      agent.pubkey,
      getAgentObserverSnapshot(agent.pubkey, true).events,
    );
  }
}

export function subscribeRecentAgentTurnFailures(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function isTopLevelFailure(failure: RecentAgentTurnFailure): boolean {
  return (
    failure.rootEventId !== null &&
    failure.rootEventId ===
      failure.triggeringEventIds[failure.triggeringEventIds.length - 1]
  );
}

export function getRecentAgentTurnFailures(
  channelId: string | null | undefined,
  rootEventId: string | null = null,
): RecentAgentTurnFailure[] {
  if (!channelId) return EMPTY_FAILURES;
  const cacheKey = `${channelId}:${rootEventId ?? "*"}`;
  const cached = cachedByScope.get(cacheKey);
  if (cached) return cached;

  const failures: RecentAgentTurnFailure[] = [];
  for (const agentFailures of failuresByAgent.values()) {
    for (const failure of agentFailures.values()) {
      if (
        failure.channelId === channelId &&
        (rootEventId === null
          ? isTopLevelFailure(failure)
          : failure.rootEventId === rootEventId)
      ) {
        failures.push(failure);
      }
    }
  }
  failures.sort((left, right) => right.timestamp.localeCompare(left.timestamp));
  const result = failures.length > 0 ? failures : EMPTY_FAILURES;
  cachedByScope.set(cacheKey, result);
  return result;
}

export function useRecentAgentTurnFailures(
  channelId: string | null | undefined,
  rootEventId: string | null = null,
) {
  const getSnapshot = React.useCallback(
    () => getRecentAgentTurnFailures(channelId, rootEventId),
    [channelId, rootEventId],
  );
  return React.useSyncExternalStore(
    subscribeRecentAgentTurnFailures,
    getSnapshot,
  );
}

export function createRecentAgentTurnFailuresObserverListener(
  agents: readonly { pubkey: string; status: string }[],
): (update?: AgentObserverStoreUpdate) => void {
  const activeAgentPubkeys = new Set(
    agents
      .filter(
        (agent) => agent.status === "running" || agent.status === "deployed",
      )
      .map((agent) => normalizePubkey(agent.pubkey)),
  );

  return (update?: AgentObserverStoreUpdate) => {
    if (
      !update ||
      !activeAgentPubkeys.has(normalizePubkey(update.agentPubkey))
    ) {
      return;
    }
    syncRecentAgentTurnFailuresFromEvents(update.agentPubkey, [
      ...update.events,
    ]);
  };
}

export function useRecentAgentTurnFailuresBridge(
  agents: readonly { pubkey: string; status: string }[],
) {
  React.useEffect(() => {
    syncRecentAgentTurnFailuresFromObserver(agents);
    return subscribeAgentObserverStore(
      createRecentAgentTurnFailuresObserverListener(agents),
    );
  }, [agents]);
}

export function resetRecentAgentTurnFailuresStore() {
  failuresByAgent.clear();
  turnContextsByAgent.clear();
  lastProcessedByAgent.clear();
  notify();
}

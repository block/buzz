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
import {
  advanceObserverChannelProgress,
  type ObserverChannelProgress,
} from "./observerChannelProgress";

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
  | "stopped"
  | "unknown";

export type RecentAgentTurnFailure = TurnContext & {
  agentPubkey: string;
  turnId: string;
  error: string;
  disposition: TurnFailureDisposition;
  attempt: number | null;
  respawnScheduled?: boolean;
  timestamp: string;
};

type StoredTurnFailure = {
  failure: RecentAgentTurnFailure;
  event: ObserverEvent;
};

const failuresByAgent = new Map<string, Map<string, StoredTurnFailure>>();
const turnContextsByAgent = new Map<string, Map<string, TurnContext>>();
const lastProcessedByAgent = new Map<
  string,
  Map<string, ObserverChannelProgress>
>();
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
    rootEventId,
    parentEventId,
    triggeringEventIds: ids,
  };
}

function failureKey(context: TurnContext): string {
  return `${context.channelId}:${context.rootEventId ?? `unknown:${context.triggeringEventIds.join(",")}`}`;
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
      return "unknown";
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

function removeCoveredFailures(
  agentKey: string,
  context: TurnContext,
  startedEvent: ObserverEvent,
): boolean {
  const failures = failuresByAgent.get(agentKey);
  if (!failures) return false;
  const coveredIds = new Set(context.triggeringEventIds);
  let changed = false;
  for (const [key, { failure, event }] of failures) {
    if (failure.channelId !== context.channelId) continue;
    if (compareObserverEvents(startedEvent, event) <= 0) continue;
    const covered =
      failure.triggeringEventIds.length > 0
        ? failure.triggeringEventIds.every((id) => coveredIds.has(id))
        : failure.rootEventId !== null &&
          failure.rootEventId === context.rootEventId;
    if (covered) {
      failures.delete(key);
      changed = true;
    }
  }
  if (failures.size === 0) failuresByAgent.delete(agentKey);
  return changed;
}

function setFailure(
  agentKey: string,
  failure: RecentAgentTurnFailure,
  event: ObserverEvent,
) {
  let failures = failuresByAgent.get(agentKey);
  if (!failures) {
    failures = new Map();
    failuresByAgent.set(agentKey, failures);
  }
  const key = failureKey(failure);
  const prior = failures.get(key);
  if (prior && compareObserverEvents(event, prior.event) <= 0) return false;
  failures.delete(key);
  failures.set(key, { failure, event });
  while (failures.size > MAX_FAILURES_PER_AGENT) {
    // Arrival order is not age when shutdown promotes its final failure.
    // Draining older buffered failures must not evict that newest terminal.
    let oldest: [string, StoredTurnFailure] | undefined;
    for (const entry of failures) {
      if (
        !oldest ||
        compareObserverEvents(entry[1].event, oldest[1].event) < 0
      ) {
        oldest = entry;
      }
    }
    if (!oldest) break;
    failures.delete(oldest[0]);
  }
  return true;
}

function processEvent(agentPubkey: string, event: ObserverEvent): boolean {
  const agentKey = normalizePubkey(agentPubkey);
  const channelKey = event.channelId ?? "\u0000null-channel";
  let watermarks = lastProcessedByAgent.get(agentKey);
  const admission = advanceObserverChannelProgress(
    watermarks?.get(channelKey),
    event,
  );
  if (!admission) return false;
  if (!watermarks) {
    watermarks = new Map();
    lastProcessedByAgent.set(agentKey, watermarks);
  }
  watermarks.set(channelKey, admission.progress);
  if (!admission.apply) return false;

  const turnId = event.turnId;
  if (event.kind === "turn_started" && turnId) {
    const context = contextFromEvent(event);
    if (!context) return false;
    rememberTurnContext(agentKey, turnId, context);
    return removeCoveredFailures(agentKey, context, event);
  }

  if (event.kind !== "turn_error" && event.kind !== "agent_panic") {
    return false;
  }

  const payload = asRecord(event.payload);
  const startedContext = turnId
    ? turnContextsByAgent.get(agentKey)?.get(turnId)
    : null;
  const reportedContext = contextFromEvent(event);
  const context =
    reportedContext && startedContext
      ? {
          ...reportedContext,
          triggeringEventIds:
            reportedContext.triggeringEventIds.length > 0
              ? reportedContext.triggeringEventIds
              : startedContext.triggeringEventIds,
          rootEventId:
            reportedContext.rootEventId ?? startedContext.rootEventId,
          parentEventId:
            reportedContext.parentEventId ?? startedContext.parentEventId,
        }
      : (reportedContext ?? startedContext);
  if (!context || !turnId) return false;
  const rawError = asString(payload.error) ?? "Unknown error";
  const numericAttempt = Number(payload.attempt);
  return setFailure(
    agentKey,
    {
      ...context,
      agentPubkey,
      turnId,
      error: friendlyTurnErrorCopy(rawError, payload.code),
      disposition: disposition(payload.disposition),
      respawnScheduled:
        typeof payload.respawnScheduled === "boolean"
          ? payload.respawnScheduled
          : undefined,
      attempt:
        Number.isInteger(numericAttempt) && numericAttempt > 0
          ? numericAttempt
          : null,
      timestamp: event.timestamp,
    },
    event,
  );
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
    for (const { failure } of agentFailures.values()) {
      if (
        failure.channelId === channelId &&
        (rootEventId === null
          ? failure.rootEventId === null || isTopLevelFailure(failure)
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

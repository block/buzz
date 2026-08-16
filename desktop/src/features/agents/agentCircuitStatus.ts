import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ObserverEvent } from "./ui/agentSessionTypes";
import { isObserverEventAfter } from "./lib/observerEventOrdering";

export type AgentCircuitStatus = {
  isOpen: boolean;
  message: string | null;
  channelId: string | null;
  timestamp: string | null;
};

const IDLE_CIRCUIT_STATUS: AgentCircuitStatus = {
  isOpen: false,
  message: null,
  channelId: null,
  timestamp: null,
};

// Latest known circuit-breaker state per (agent, slot). Slots are independent
// prompt lanes within one managed-agent process (see SlotCircuit in
// buzz-acp/src/lib.rs) — a recovery on slot 1 must not clear a still-open
// circuit on slot 0, so state is tracked per slot and the derived per-agent
// status reports "open" if ANY slot is open.
type CircuitSlotState = {
  isOpen: boolean;
  message: string;
  channelId: string | null;
  timestamp: string;
  seq: number;
};
const circuitStateBySlot = new Map<string, CircuitSlotState>();

// Reference-stable per-agent snapshots for useSyncExternalStore, invalidated
// only when that agent's circuit state actually changes.
const circuitStatusCache = new Map<string, AgentCircuitStatus>();

function circuitSlotKey(
  agentPubkey: string,
  agentIndex: number | null | undefined,
): string {
  return `${normalizePubkey(agentPubkey)}:${agentIndex ?? 0}`;
}

/**
 * Fold a circuit_open/circuit_recovered event into per-slot circuit state.
 * Ignores any other event kind and any event that doesn't sort strictly after
 * the slot's currently-stored state, so a late/replayed frame can't regress
 * an already-newer status. Returns true if state actually changed.
 */
export function applyCircuitEvent(
  agentPubkey: string,
  event: ObserverEvent,
): boolean {
  if (event.kind !== "circuit_open" && event.kind !== "circuit_recovered") {
    return false;
  }
  const key = circuitSlotKey(agentPubkey, event.agentIndex);
  const existing = circuitStateBySlot.get(key);
  if (existing && !isObserverEventAfter(event, existing)) {
    return false;
  }
  const payload = event.payload as { error?: unknown } | null;
  const message =
    typeof payload?.error === "string"
      ? payload.error
      : event.kind === "circuit_open"
        ? "Agent suspended (repeated crashes)"
        : "Agent recovered";
  circuitStateBySlot.set(key, {
    isOpen: event.kind === "circuit_open",
    message,
    channelId: event.channelId ?? null,
    timestamp: event.timestamp,
    seq: event.seq,
  });
  circuitStatusCache.delete(normalizePubkey(agentPubkey));
  return true;
}

/**
 * Derived circuit-breaker status for one agent: open if ANY of its slots is
 * currently open. Reference-stable while unchanged (useSyncExternalStore-safe
 * — see the React.memo/useSyncExternalStore gotcha in this repo's CLAUDE.md).
 */
export function getAgentCircuitStatus(
  agentPubkey?: string | null,
): AgentCircuitStatus {
  if (!agentPubkey) {
    return IDLE_CIRCUIT_STATUS;
  }
  const key = normalizePubkey(agentPubkey);
  const cached = circuitStatusCache.get(key);
  if (cached) {
    return cached;
  }
  const prefix = `${key}:`;
  let openSlot: CircuitSlotState | null = null;
  for (const [slotKey, slot] of circuitStateBySlot) {
    if (!slotKey.startsWith(prefix) || !slot.isOpen) continue;
    if (!openSlot || isObserverEventAfter(slot, openSlot)) {
      openSlot = slot;
    }
  }
  const status: AgentCircuitStatus = openSlot
    ? {
        isOpen: true,
        message: openSlot.message,
        channelId: openSlot.channelId,
        timestamp: openSlot.timestamp,
      }
    : IDLE_CIRCUIT_STATUS;
  circuitStatusCache.set(key, status);
  return status;
}

/** Clears all per-agent circuit state. Called from resetAgentObserverStore so
 * circuit state can't leak across community switches. */
export function resetCircuitState() {
  circuitStateBySlot.clear();
  circuitStatusCache.clear();
}

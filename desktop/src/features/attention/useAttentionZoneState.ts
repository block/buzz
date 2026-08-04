import * as React from "react";

import {
  persistableZoneState,
  pruneZoneState,
  type ZoneStateEntry,
  type ZoneStateMap,
} from "@/features/attention/lib/attention";

const EMPTY_HOLD_IDS: ReadonlySet<string> = new Set();

const STORAGE_KEY = "buzz-attention-zones.v1";

function storageKey(pubkey: string) {
  return `${STORAGE_KEY}:${pubkey}`;
}

function nowSeconds() {
  return Math.floor(Date.now() / 1_000);
}

function isZoneStateEntry(value: unknown): value is ZoneStateEntry {
  if (typeof value !== "object" || value === null) return false;
  const entry = value as Record<string, unknown>;
  return (
    (entry.zone === "waiting" || entry.zone === "done") &&
    typeof entry.changedAt === "number" &&
    (entry.respondedAt === undefined || typeof entry.respondedAt === "number")
  );
}

function readStoredState(key: string): ZoneStateMap {
  if (typeof window === "undefined") return {};
  const raw = window.localStorage.getItem(key);
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return {};
    const state: ZoneStateMap = {};
    for (const [id, entry] of Object.entries(parsed)) {
      if (isZoneStateEntry(entry)) {
        state[id] = entry;
      }
    }
    return pruneZoneState(state, nowSeconds());
  } catch {
    return {};
  }
}

function writeStoredState(key: string, state: ZoneStateMap) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(
    key,
    JSON.stringify(pruneZoneState(state, nowSeconds())),
  );
}

/**
 * Per-user, per-device zone state for Attention items, keyed by conversation id.
 * Prototype persistence: localStorage, mirroring the Inbox's local done set.
 * Durable cross-device state is a follow-up architecture decision.
 *
 * `holdIds` are items inside an undo hold window: their zone changes stay
 * in memory only and persist on commit, so quitting mid-window restores
 * the card instead of stranding it cleared with its reply unsent.
 */
export function useAttentionZoneState(
  pubkey: string | undefined,
  holdIds: ReadonlySet<string> = EMPTY_HOLD_IDS,
) {
  const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";

  const [zoneState, setZoneState] = React.useState<ZoneStateMap>(() =>
    readStoredState(storageKey(normalizedPubkey)),
  );
  const [loadedPubkey, setLoadedPubkey] = React.useState(normalizedPubkey);

  React.useEffect(() => {
    setZoneState(readStoredState(storageKey(normalizedPubkey)));
    setLoadedPubkey(normalizedPubkey);
  }, [normalizedPubkey]);

  React.useEffect(() => {
    if (loadedPubkey !== normalizedPubkey) return;
    writeStoredState(
      storageKey(normalizedPubkey),
      persistableZoneState(zoneState, holdIds),
    );
  }, [holdIds, loadedPubkey, normalizedPubkey, zoneState]);

  const markWaiting = React.useCallback((id: string) => {
    setZoneState((prev) => ({
      ...prev,
      [id]: { zone: "waiting", changedAt: nowSeconds() },
    }));
  }, []);

  const markDone = React.useCallback((id: string) => {
    setZoneState((prev) => ({
      ...prev,
      [id]: { zone: "done", changedAt: nowSeconds() },
    }));
  }, []);

  /**
   * Record that the parking action's reply published. Called at commit,
   * after the send resolves — the reactivated card uses it to show the
   * reply went out, so the user does not answer twice.
   */
  const markResponded = React.useCallback((id: string) => {
    setZoneState((prev) => {
      const entry = prev[id];
      if (!entry) return prev;
      return { ...prev, [id]: { ...entry, respondedAt: nowSeconds() } };
    });
  }, []);

  const restore = React.useCallback((id: string) => {
    setZoneState((prev) => {
      if (!(id in prev)) return prev;
      const next = { ...prev };
      delete next[id];
      return next;
    });
  }, []);

  /**
   * Put back the exact entry captured before an action, `changedAt`
   * included. Re-marking would stamp a fresh time and defeat the
   * reactivation rule: undoing an action on a reactivated card would
   * hide it in its old zone instead of returning it to Needs Me.
   */
  const restoreEntry = React.useCallback(
    (id: string, entry: ZoneStateEntry | undefined) => {
      setZoneState((prev) => {
        if (!entry) {
          if (!(id in prev)) return prev;
          const next = { ...prev };
          delete next[id];
          return next;
        }
        return { ...prev, [id]: entry };
      });
    },
    [],
  );

  return {
    markDone,
    markResponded,
    markWaiting,
    restore,
    restoreEntry,
    zoneState,
  };
}

export type AttentionState = ReturnType<typeof useAttentionZoneState>;

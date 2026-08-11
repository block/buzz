import * as React from "react";

import type { AskType } from "@/features/attention/lib/attention";

const STORAGE_KEY = "buzz-attention-badge-overrides.v1";
const MAX_OVERRIDE_ENTRIES = 500;

const ASK_TYPES: ReadonlySet<string> = new Set([
  "decision",
  "approval",
  "question",
  "review",
  "blocked",
  "headsUp",
]);

export type BadgeOverrideMap = Record<string, AskType>;

function storageKey(pubkey: string) {
  return `${STORAGE_KEY}:${pubkey}`;
}

function readStoredOverrides(key: string): BadgeOverrideMap {
  if (typeof window === "undefined") return {};
  const raw = window.localStorage.getItem(key);
  if (!raw) return {};
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return {};
    const state: BadgeOverrideMap = {};
    for (const [id, type] of Object.entries(parsed)) {
      if (typeof type === "string" && ASK_TYPES.has(type)) {
        state[id] = type as AskType;
      }
    }
    return state;
  } catch {
    return {};
  }
}

/** Newest insertions win when the map exceeds the cap. */
function capOverrides(state: BadgeOverrideMap): BadgeOverrideMap {
  const entries = Object.entries(state);
  if (entries.length <= MAX_OVERRIDE_ENTRIES) {
    return state;
  }
  return Object.fromEntries(entries.slice(-MAX_OVERRIDE_ENTRIES));
}

function writeStoredOverrides(key: string, state: BadgeOverrideMap) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(key, JSON.stringify(state));
}

/**
 * Per-user, per-device badge corrections for Attention items, keyed by
 * conversation id. Purely local and deterministic — reclassifying a card
 * never posts anything. Mirrors the zone-state persistence pattern.
 */
export function useBadgeOverrides(pubkey: string | undefined) {
  const normalizedPubkey = pubkey?.trim().toLowerCase() ?? "";

  const [overrides, setOverrides] = React.useState<BadgeOverrideMap>(() =>
    readStoredOverrides(storageKey(normalizedPubkey)),
  );
  const [loadedPubkey, setLoadedPubkey] = React.useState(normalizedPubkey);

  React.useEffect(() => {
    setOverrides(readStoredOverrides(storageKey(normalizedPubkey)));
    setLoadedPubkey(normalizedPubkey);
  }, [normalizedPubkey]);

  React.useEffect(() => {
    if (loadedPubkey !== normalizedPubkey) return;
    writeStoredOverrides(storageKey(normalizedPubkey), overrides);
  }, [loadedPubkey, normalizedPubkey, overrides]);

  const setOverride = React.useCallback((id: string, type: AskType) => {
    setOverrides((prev) => capOverrides({ ...prev, [id]: type }));
  }, []);

  return { overrides, setOverride };
}

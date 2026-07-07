import * as React from "react";

import {
  isTimeoutActive,
  parseTimeoutRejection,
} from "@/features/moderation/lib/timeout";

/**
 * Process-wide store for the member's current community timeout, learned
 * reactively from send rejections (there is no proactive read in v1). A single
 * value suffices: the desktop app is bound to one community per relay
 * connection, and a timeout blocks every channel's writes at the auth seam.
 *
 * Kept as a tiny external store rather than React Query cache because the value
 * is written from a mutation's error path and read by the composer with a live
 * countdown — an imperative setter plus `useSyncExternalStore` is the smallest
 * primitive that fits both.
 */

let expiresAtMs: number | null = null;
// `true` once a timeout is recorded, until it expires or a send is accepted.
// Distinguishes "not timed out" (null + inactive) from "timed out, unknown
// expiry" (active with a null `expiresAtMs`).
let active = false;
const listeners = new Set<() => void>();

// Cached snapshot so `useSyncExternalStore` gets a referentially stable value
// between changes (returning a fresh object each read would loop forever).
let snapshot: TimeoutState = { active: false, expiresAtMs: null };

function emit() {
  snapshot = { active, expiresAtMs };
  for (const listener of listeners) {
    listener();
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Inspect a relay send-rejection message. If it is a timeout refusal, record
 * the timeout and return `true` so the caller can suppress its generic error
 * surface. Any other rejection returns `false` and is left untouched.
 */
export function recordTimeoutFromRejection(
  message: string | null | undefined,
): boolean {
  const rejection = parseTimeoutRejection(message);
  if (!rejection) {
    return false;
  }
  expiresAtMs = rejection.expiresAtMs;
  active = true;
  emit();
  return true;
}

/** Clear the timeout — called when a send is accepted (the block is lifted). */
export function clearTimeoutState(): void {
  if (!active && expiresAtMs === null) {
    return;
  }
  expiresAtMs = null;
  active = false;
  emit();
}

export type TimeoutState = {
  /** True while the member is timed out (write-blocked). */
  active: boolean;
  /** Expiry in epoch ms, or null when the relay gave no parseable timestamp. */
  expiresAtMs: number | null;
};

const INACTIVE: TimeoutState = { active: false, expiresAtMs: null };

function currentState(state: TimeoutState, nowMs: number): TimeoutState {
  if (!state.active) {
    return INACTIVE;
  }
  if (!isTimeoutActive(state.expiresAtMs, nowMs)) {
    // A known expiry has passed; collapse to inactive so the next read is
    // clean even before a send re-probes the block.
    return INACTIVE;
  }
  return state;
}

/**
 * Subscribe to the timeout state. Re-renders on record/clear and, while a
 * known-expiry timeout is active, ticks once a second so a countdown UI stays
 * live and auto-clears exactly at expiry.
 */
export function useTimeoutState(): TimeoutState {
  const [nowMs, setNowMs] = React.useState(() => Date.now());
  const state = React.useSyncExternalStore(
    subscribe,
    () => snapshot,
    () => INACTIVE,
  );

  React.useEffect(() => {
    if (!state.active || state.expiresAtMs === null) {
      return;
    }
    setNowMs(Date.now());
    const id = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => {
      window.clearInterval(id);
    };
  }, [state.active, state.expiresAtMs]);

  return currentState(state, nowMs);
}

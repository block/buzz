import * as React from "react";

import { DEFAULT_BESTIE_DUE_CHIP_HORIZON_MINUTES } from "./bestieDueCountdown";

/**
 * Device-level preference: how far out Reminder/Job due chips appear on RHS
 * category rows (and item rows). 0 = hide chips. Default 60 minutes.
 * Kept as a preference API (no popover "Due chips" control — chips belong on
 * the Reminders / Jobs RHS menu items).
 */
const STORAGE_KEY = "buzz-bestie-due-chip-horizon-minutes.v1";

export const BESTIE_DUE_CHIP_HORIZON_OPTIONS_MINUTES = [
  0, 15, 30, 60, 120,
] as const;

const listeners = new Set<() => void>();

let horizonMinutes = readStoredPreference();

function clampHorizon(value: number): number {
  if (!Number.isFinite(value) || value < 0) {
    return DEFAULT_BESTIE_DUE_CHIP_HORIZON_MINUTES;
  }
  return Math.min(24 * 60, Math.floor(value));
}

function readStoredPreference(): number {
  if (typeof window === "undefined") {
    return DEFAULT_BESTIE_DUE_CHIP_HORIZON_MINUTES;
  }
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw == null) return DEFAULT_BESTIE_DUE_CHIP_HORIZON_MINUTES;
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) return DEFAULT_BESTIE_DUE_CHIP_HORIZON_MINUTES;
    return clampHorizon(parsed);
  } catch {
    return DEFAULT_BESTIE_DUE_CHIP_HORIZON_MINUTES;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): number {
  return horizonMinutes;
}

function getServerSnapshot(): number {
  return DEFAULT_BESTIE_DUE_CHIP_HORIZON_MINUTES;
}

/** Update the due-chip horizon (minutes; 0 hides chips). */
export function setBestieDueChipHorizonMinutes(minutes: number): void {
  horizonMinutes = clampHorizon(minutes);
  try {
    window.localStorage.setItem(STORAGE_KEY, String(horizonMinutes));
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }
  for (const listener of listeners) {
    listener();
  }
}

/** Minutes until due when countdown chips appear (0 = off). */
export function useBestieDueChipHorizonMinutes(): number {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

export const BESTIE_DUE_CHIP_HORIZON_STORAGE_KEY = STORAGE_KEY;

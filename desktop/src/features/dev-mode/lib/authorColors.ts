import * as React from "react";

import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Deterministic per-author name colors for developer mode, with optional
 * per-pubkey hex overrides persisted in localStorage (set via the command
 * palette). A synced profile color field would need schema/event changes, so
 * overrides are device-local for now.
 */

/** Mid-lightness hues that stay readable on both dark and light themes. */
export const AUTHOR_COLOR_PALETTE = [
  "#e5484d", // red
  "#f76b15", // orange
  "#ffb224", // amber
  "#46a758", // green
  "#12a594", // teal
  "#0091ff", // blue
  "#3e63dd", // indigo
  "#6e56cf", // violet
  "#8e4ec6", // purple
  "#e93d82", // pink
  "#05a2c2", // cyan
  "#978365", // bronze
] as const;

const STORAGE_KEY = "buzz.devMode.nameColors";
const HEX_COLOR_RE = /^#[0-9a-f]{6}$/;

const listeners = new Set<() => void>();

let overrides = readStoredOverrides();

function readStoredOverrides(): Record<string, string> {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return {};
    const valid: Record<string, string> = {};
    for (const [pubkey, color] of Object.entries(parsed)) {
      if (typeof color === "string" && HEX_COLOR_RE.test(color)) {
        valid[pubkey] = color;
      }
    }
    return valid;
  } catch {
    return {};
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): Record<string, string> {
  return overrides;
}

export function normalizeHexColor(value: string): string | null {
  const trimmed = value.trim().toLowerCase();
  const withHash = trimmed.startsWith("#") ? trimmed : `#${trimmed}`;
  const expanded = /^#[0-9a-f]{3}$/.test(withHash)
    ? `#${withHash[1]}${withHash[1]}${withHash[2]}${withHash[2]}${withHash[3]}${withHash[3]}`
    : withHash;
  return HEX_COLOR_RE.test(expanded) ? expanded : null;
}

export function setNameColorOverride(
  pubkey: string,
  color: string | null,
): void {
  const key = normalizePubkey(pubkey);
  const next = { ...overrides };
  if (color === null) {
    delete next[key];
  } else {
    const normalized = normalizeHexColor(color);
    if (!normalized) return;
    next[key] = normalized;
  }
  overrides = next;
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }
  for (const listener of listeners) {
    listener();
  }
}

/** FNV-1a — stable across sessions, unlike per-run hashing. */
function hashPubkey(pubkey: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < pubkey.length; index += 1) {
    hash ^= pubkey.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}

export function defaultAuthorColor(pubkey: string): string {
  const key = normalizePubkey(pubkey);
  return AUTHOR_COLOR_PALETTE[hashPubkey(key) % AUTHOR_COLOR_PALETTE.length];
}

export type AuthorColorResolver = (pubkey: string) => string;

export function useAuthorColorResolver(): AuthorColorResolver {
  const current = React.useSyncExternalStore(
    subscribe,
    getSnapshot,
    getSnapshot,
  );
  return React.useCallback<AuthorColorResolver>(
    (pubkey) => current[normalizePubkey(pubkey)] ?? defaultAuthorColor(pubkey),
    [current],
  );
}

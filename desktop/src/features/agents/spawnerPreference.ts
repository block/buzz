import React from "react";

/**
 * The spawners this device deploys server-hosted agents to.
 *
 * A list, not a single value: an owner may keep a beefy box for agents that
 * need it and a cheap VPS for the rest, and each agent picks where it runs.
 *
 * Local-only, like [`trustedSpawners`](./trustedSpawners.ts). Which servers you
 * deploy to is a per-device operational choice — a laptop on a VPN may reach a
 * spawner a phone cannot.
 *
 * Only pubkeys are stored. A spawner is addressed entirely through the relay
 * (specs carry a `spawner` tag, status comes back `#p`-tagged), so it can run on
 * any host, behind NAT, anywhere with outbound WebSocket — there is no URL for a
 * client to know, cache, or get wrong.
 */
const STORAGE_KEY = "buzz:spawner-pubkeys";

/** Pre-list key, read once so an existing single-spawner setup carries over. */
const LEGACY_STORAGE_KEY = "buzz:spawner-pubkey";

const listeners = new Set<() => void>();

/// Declared before `spawners` because `readStored()` returns it: the reverse
/// order is a temporal-dead-zone crash at module load, which is exactly what
/// happens on a device with nothing stored yet.
const EMPTY: readonly string[] = [];

let spawners: readonly string[] = readStored();

function isPubkeyHex(value: string): boolean {
  return value.length === 64 && /^[0-9a-f]+$/i.test(value);
}

function readStored(): readonly string[] {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed: unknown = JSON.parse(raw);
      if (Array.isArray(parsed)) {
        return dedupe(
          parsed.filter(
            (v): v is string => typeof v === "string" && isPubkeyHex(v),
          ),
        );
      }
      return EMPTY;
    }
    // Migrate the single-value key rather than silently dropping a spawner the
    // user already connected to.
    const legacy = window.localStorage.getItem(LEGACY_STORAGE_KEY);
    if (legacy && isPubkeyHex(legacy)) {
      const migrated = [legacy.toLowerCase()];
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(migrated));
      window.localStorage.removeItem(LEGACY_STORAGE_KEY);
      return migrated;
    }
    return EMPTY;
  } catch {
    return EMPTY;
  }
}

function dedupe(values: string[]): readonly string[] {
  return [...new Set(values.map((v) => v.toLowerCase()))];
}

function persist(next: readonly string[]): void {
  spawners = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Keep the in-memory value so this session still works.
  }
  for (const listener of listeners) listener();
}

/**
 * Connect this device to a spawner.
 *
 * Returns false when the value is not a 64-character hex pubkey, so callers can
 * show a validation message rather than silently storing nothing.
 */
export function addSpawner(pubkey: string): boolean {
  if (!isPubkeyHex(pubkey)) return false;
  const normalized = pubkey.toLowerCase();
  if (spawners.includes(normalized)) return true;
  persist([...spawners, normalized]);
  return true;
}

/**
 * Disconnect this device from a spawner.
 *
 * Local only: it stops this device managing that spawner's agents. Agents
 * already deployed keep running, because their specs still live on the relay —
 * removing them means deleting those specs.
 */
export function removeSpawner(pubkey: string): void {
  const normalized = pubkey.toLowerCase();
  if (!spawners.includes(normalized)) return;
  persist(spawners.filter((s) => s !== normalized));
}

/** Every connected spawner pubkey. */
export function getSpawners(): readonly string[] {
  return spawners;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): readonly string[] {
  return spawners;
}

function getServerSnapshot(): readonly string[] {
  return EMPTY;
}

/** Reactive view of the connected spawners. */
export function useSpawners(): readonly string[] {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

/**
 * Derive a spec slug from a display name.
 *
 * Slugs travel into container names, volume names, and log paths on the spawner
 * host, so the Rust side accepts only lowercase alphanumerics, hyphens, and
 * underscores and rejects anything else. Normalizing here means a user typing
 * "Fizz (prod)" gets `fizz-prod` instead of a validation error they cannot act
 * on. Returns null when nothing usable survives.
 */
export function slugFromName(name: string): string | null {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^[-_]+/, "")
    .replace(/-+$/, "")
    .slice(0, 64)
    // A trailing hyphen can reappear after the 64-byte truncation.
    .replace(/-+$/, "");
  return slug.length > 0 ? slug : null;
}

/**
 * Normalize a persona id to the `d` tag its kind:30175 event is published under.
 *
 * Mirrors `persona_d_tag` / `normalize_d_tag` in
 * `desktop/src-tauri/src/managed_agents/persona_events.rs`, which enforces the
 * NIP-AP grammar `^[a-z0-9][a-z0-9_-]{0,63}$`. This matters because a persona's
 * *id* and its *address on the relay* are not the same string: the built-in
 * `builtin:fizz` is published as `builtin-fizz`, since the relay rejects a `d`
 * tag containing a colon. A spec that referenced the raw id would point at a
 * persona that cannot exist, and the spawner would fail to resolve the prompt.
 */
export function personaDTag(personaId: string): string {
  let out = "";
  for (const ch of personaId.toLowerCase()) {
    out += /[a-z0-9_-]/.test(ch) ? ch : "-";
  }
  if (!/^[a-z0-9]/.test(out)) out = `a${out}`;
  return out.slice(0, 64);
}

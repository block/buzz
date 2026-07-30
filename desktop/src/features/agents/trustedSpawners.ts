import React from "react";

/**
 * The set of spawner pubkeys this user has approved.
 *
 * # Why this exists
 *
 * Signing a NIP-OA attestation admits an agent pubkey to the relay under *this
 * user's* membership. A spawner that obtains a signature can run an agent that
 * reads the user's channels. So the first request from any spawner must be an
 * explicit, human decision — auto-signing for an unknown pubkey would turn a
 * single malicious kind:24201 frame into silent access.
 *
 * Trust is remembered per spawner so an owner running several agents on their
 * own VPS is prompted once, not once per agent.
 *
 * Deliberately local-only, unlike most Buzz preferences which roam via NIP-78
 * kind:30078. Syncing this would mean approving a spawner on a phone silently
 * authorizes it on a laptop, which is the opposite of what a trust decision
 * should do — each device should vouch for itself.
 */
const STORAGE_KEY = "buzz:trusted-spawners";

const listeners = new Set<() => void>();

let trusted: ReadonlySet<string> = readStored();

const EMPTY: ReadonlySet<string> = new Set<string>();

function readStored(): ReadonlySet<string> {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return new Set<string>();
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set<string>();
    return new Set(
      parsed.filter(
        (value): value is string =>
          typeof value === "string" && isPubkeyHex(value),
      ),
    );
  } catch {
    return new Set<string>();
  }
}

function persist(next: ReadonlySet<string>): void {
  trusted = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify([...next]));
  } catch {
    // A full or unavailable localStorage must not break the prompt. The
    // decision still applies to this request; the user is asked again next
    // time, which fails toward asking rather than toward trusting.
  }
  for (const listener of listeners) listener();
}

function isPubkeyHex(value: string): boolean {
  return value.length === 64 && /^[0-9a-f]+$/i.test(value);
}

/** Whether the user has approved this spawner. */
export function isSpawnerTrusted(spawnerPubkey: string): boolean {
  return trusted.has(spawnerPubkey.toLowerCase());
}

/** Record approval for a spawner. */
export function trustSpawner(spawnerPubkey: string): void {
  const normalized = spawnerPubkey.toLowerCase();
  if (!isPubkeyHex(normalized) || trusted.has(normalized)) return;
  persist(new Set([...trusted, normalized]));
}

/**
 * Revoke approval for a spawner.
 *
 * Revoking does not retract attestations already signed — a NIP-OA tag, once
 * issued, is valid until the agent's identity is archived. It only stops future
 * requests from being auto-approved. Tearing down the agents themselves means
 * deleting their specs.
 */
export function revokeSpawner(spawnerPubkey: string): void {
  const normalized = spawnerPubkey.toLowerCase();
  if (!trusted.has(normalized)) return;
  const next = new Set(trusted);
  next.delete(normalized);
  persist(next);
}

/** Clear every trust decision. Used when switching identities. */
export function resetTrustedSpawners(): void {
  if (trusted.size === 0) return;
  persist(new Set<string>());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): ReadonlySet<string> {
  return trusted;
}

function getServerSnapshot(): ReadonlySet<string> {
  return EMPTY;
}

/** Reactive view of the approved spawner set. */
export function useTrustedSpawners(): ReadonlySet<string> {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

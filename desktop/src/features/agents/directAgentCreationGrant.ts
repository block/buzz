import * as React from "react";

export const DIRECT_AGENT_CREATION_GRANTS_STORAGE_KEY =
  "buzz.agents.directCreationGrants";

const HEX_PUBKEY = /^[0-9a-f]{64}$/;
const listeners = new Set<() => void>();
const EMPTY_GRANTS: readonly string[] = [];
const grantsByOwner = new Map<string, readonly string[]>();

function normalizePubkey(value: string): string | null {
  const normalized = value.trim().toLowerCase();
  return HEX_PUBKEY.test(normalized) ? normalized : null;
}

export function parseDirectAgentCreationGrants(
  value: string | null | undefined,
): string[] {
  if (!value) return [];
  try {
    const parsed: unknown = JSON.parse(value);
    if (!Array.isArray(parsed)) return [];
    return [
      ...new Set(
        parsed.flatMap((item) => {
          if (typeof item !== "string") return [];
          const normalized = normalizePubkey(item);
          return normalized ? [normalized] : [];
        }),
      ),
    ].sort();
  } catch {
    return [];
  }
}

function storageKey(ownerPubkey: string): string {
  return `${DIRECT_AGENT_CREATION_GRANTS_STORAGE_KEY}.${ownerPubkey}`;
}

function readStoredGrants(ownerPubkey: string): string[] {
  try {
    return parseDirectAgentCreationGrants(
      globalThis.localStorage?.getItem(storageKey(ownerPubkey)),
    );
  } catch {
    return [];
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getDirectAgentCreationGrants(
  ownerPubkey: string,
): readonly string[] {
  const owner = normalizePubkey(ownerPubkey);
  if (!owner) return EMPTY_GRANTS;
  const cached = grantsByOwner.get(owner);
  if (cached) return cached;
  const stored = readStoredGrants(owner);
  grantsByOwner.set(owner, stored);
  return stored;
}

export function hasDirectAgentCreationGrant(
  ownerPubkey: string,
  pubkey: string,
): boolean {
  const normalized = normalizePubkey(pubkey);
  return (
    normalized !== null &&
    getDirectAgentCreationGrants(ownerPubkey).includes(normalized)
  );
}

export function setDirectAgentCreationGrant(
  ownerPubkey: string,
  pubkey: string,
  enabled: boolean,
): void {
  const owner = normalizePubkey(ownerPubkey);
  const normalized = normalizePubkey(pubkey);
  if (!owner || !normalized) return;
  const current = getDirectAgentCreationGrants(owner);
  const next = new Set(current);
  if (enabled) next.add(normalized);
  else next.delete(normalized);
  const sorted = [...next].sort();
  if (JSON.stringify(sorted) === JSON.stringify(current)) return;
  try {
    globalThis.localStorage?.setItem(storageKey(owner), JSON.stringify(sorted));
  } catch {
    // Permission mutations fail closed. Keep the prior snapshot so a failed
    // grant or revocation is visible to the owner instead of pretending it
    // persisted.
    return;
  }
  grantsByOwner.set(owner, sorted);
  for (const listener of listeners) listener();
}

export function useDirectAgentCreationGrants(
  ownerPubkey: string,
): readonly string[] {
  return React.useSyncExternalStore(
    subscribe,
    () => getDirectAgentCreationGrants(ownerPubkey),
    () => EMPTY_GRANTS,
  );
}

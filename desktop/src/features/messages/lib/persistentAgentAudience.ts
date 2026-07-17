import * as React from "react";

const ENABLED_STORAGE_KEY = "buzz:keep-addressed-agents-active";
const AUDIENCES_STORAGE_KEY = "buzz:persistent-agent-audiences:v2";

const listeners = new Set<() => void>();
const revisions = new Map<string, number>();
let revisionClock = 0;
let defaultRevision = 0;
let generation = 0;
let enabled = readEnabled();
let audiences = readAudiences();
let snapshot = buildSnapshot();

export type PersistentAgentAudienceSnapshot = Readonly<{
  enabled: boolean;
  audiences: Readonly<Record<string, readonly string[]>>;
  generation: number;
}>;

type PersistentAgentAudienceScopeInput = {
  ownerPubkey: string;
  channelId: string;
  threadRootId?: string | null;
};

function normalizePubkeys(pubkeys: Iterable<string>): string[] {
  return [
    ...new Set([...pubkeys].map((pubkey) => pubkey.trim().toLowerCase())),
  ].filter((pubkey) => /^[0-9a-f]{64}$/.test(pubkey));
}

function readEnabled(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(ENABLED_STORAGE_KEY) === "1";
  } catch {
    return false;
  }
}

function readAudiences(): Record<string, string[]> {
  if (typeof window === "undefined") return {};
  try {
    const parsed: unknown = JSON.parse(
      window.localStorage.getItem(AUDIENCES_STORAGE_KEY) ?? "{}",
    );
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
      return {};

    const result: Record<string, string[]> = {};
    for (const [scope, value] of Object.entries(parsed)) {
      if (scope && Array.isArray(value)) {
        result[scope] = normalizePubkeys(
          value.filter((entry): entry is string => typeof entry === "string"),
        );
      }
    }
    return result;
  } catch {
    return {};
  }
}

function buildSnapshot(): PersistentAgentAudienceSnapshot {
  return { enabled, audiences, generation };
}

function emit(): void {
  snapshot = buildSnapshot();
  for (const listener of listeners) listener();
}

function persistAudiences(): void {
  try {
    window.localStorage.setItem(
      AUDIENCES_STORAGE_KEY,
      JSON.stringify(audiences),
    );
  } catch {
    // Persistence is best-effort; the live session still uses in-memory state.
  }
}

function advanceRevision(scope: string): void {
  revisionClock += 1;
  revisions.set(scope, revisionClock);
}

export function setPersistentAgentAudienceEnabled(nextEnabled: boolean): void {
  if (enabled === nextEnabled) return;
  enabled = nextEnabled;
  if (!nextEnabled) {
    generation += 1;
    revisionClock += 1;
    defaultRevision = revisionClock;
    revisions.clear();
    audiences = {};
    persistAudiences();
  }
  try {
    window.localStorage.setItem(ENABLED_STORAGE_KEY, nextEnabled ? "1" : "0");
  } catch {
    // Persistence is best-effort.
  }
  emit();
}

export function getPersistentAgentAudienceScope({
  ownerPubkey,
  channelId,
  threadRootId = null,
}: PersistentAgentAudienceScopeInput): string | null {
  const owner = ownerPubkey.trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(owner) || !channelId) return null;
  if (!threadRootId) return null;
  return `${owner}:${channelId}:thread:${threadRootId}`;
}

export function getPersistentAgentAudienceGeneration(): number {
  return generation;
}

export function getPersistentAgentAudienceRevision(scope: string): number {
  return revisions.get(scope) ?? defaultRevision;
}

export function initializePersistentAgentAudience(
  scope: string,
  pubkeys: Iterable<string>,
): void {
  if (!enabled || !scope || Object.hasOwn(audiences, scope)) return;
  setPersistentAgentAudience(scope, pubkeys);
}

export function setPersistentAgentAudience(
  scope: string,
  pubkeys: Iterable<string>,
): void {
  if (!scope) return;
  const normalized = normalizePubkeys(pubkeys);
  const current = audiences[scope];
  if (
    current !== undefined &&
    current.length === normalized.length &&
    current.every((pubkey, index) => pubkey === normalized[index])
  ) {
    return;
  }

  audiences = { ...audiences, [scope]: normalized };
  advanceRevision(scope);
  persistAudiences();
  emit();
}

export function promotePersistentAgentAudience({
  expectedGeneration,
  expectedRevision,
  explicitAgentPubkeys,
  scope,
}: {
  expectedGeneration: number;
  expectedRevision: number | null;
  explicitAgentPubkeys: string[];
  scope: string | null;
}): void {
  if (
    !enabled ||
    expectedGeneration !== generation ||
    !scope ||
    (expectedRevision !== null &&
      getPersistentAgentAudienceRevision(scope) !== expectedRevision)
  ) {
    return;
  }
  setPersistentAgentAudience(scope, [
    ...explicitAgentPubkeys,
    ...(audiences[scope] ?? []),
  ]);
}

export function removePersistentAgentAudienceMember(
  scope: string,
  pubkey: string,
): void {
  setPersistentAgentAudience(
    scope,
    (audiences[scope] ?? []).filter(
      (candidate) => candidate !== pubkey.trim().toLowerCase(),
    ),
  );
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): PersistentAgentAudienceSnapshot {
  return snapshot;
}

const serverSnapshot: PersistentAgentAudienceSnapshot = {
  enabled: false,
  audiences: {},
  generation: 0,
};

export function usePersistentAgentAudience(scope: string | null): {
  enabled: boolean;
  pubkeys: readonly string[];
  generation: number;
  revision: number;
  setEnabled: (enabled: boolean) => void;
  promotePubkeys: typeof promotePersistentAgentAudience;
  removePubkey: (pubkey: string) => void;
  clear: () => void;
  initialize: (pubkeys: Iterable<string>) => void;
} {
  const state = React.useSyncExternalStore(
    subscribe,
    getSnapshot,
    () => serverSnapshot,
  );
  const resolvedScope = scope ?? "";
  return {
    enabled: state.enabled,
    pubkeys: resolvedScope ? (state.audiences[resolvedScope] ?? []) : [],
    generation: state.generation,
    revision: resolvedScope
      ? getPersistentAgentAudienceRevision(resolvedScope)
      : 0,
    setEnabled: setPersistentAgentAudienceEnabled,
    promotePubkeys: promotePersistentAgentAudience,
    removePubkey: React.useCallback(
      (pubkey) => removePersistentAgentAudienceMember(resolvedScope, pubkey),
      [resolvedScope],
    ),
    clear: React.useCallback(
      () => setPersistentAgentAudience(resolvedScope, []),
      [resolvedScope],
    ),
    initialize: React.useCallback(
      (pubkeys) => initializePersistentAgentAudience(resolvedScope, pubkeys),
      [resolvedScope],
    ),
  };
}

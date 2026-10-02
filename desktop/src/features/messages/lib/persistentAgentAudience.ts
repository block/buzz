import * as React from "react";

export const MAX_IN_MEMORY_AGENT_AUDIENCES = 200;

const listeners = new Set<() => void>();
const revisions = new Map<string, number>();
const excludedPubkeysByScope = new Map<string, Set<string>>();
let revisionClock = 0;
let defaultRevision = 0;
let audiences: Record<string, string[]> = {};
let snapshot = buildSnapshot();

export type PersistentAgentAudienceSnapshot = Readonly<{
  audiences: Readonly<Record<string, readonly string[]>>;
}>;

type PersistentAgentAudienceScopeInput = {
  ownerPubkey: string;
  channelId: string;
  composerKey?: string | null;
};

function normalizePubkeys(pubkeys: Iterable<string>): string[] {
  return [
    ...new Set([...pubkeys].map((pubkey) => pubkey.trim().toLowerCase())),
  ].filter((pubkey) => /^[0-9a-f]{64}$/.test(pubkey));
}

function boundAudiences(
  value: Record<string, string[]>,
): Record<string, string[]> {
  const entries = Object.entries(value);
  return entries.length <= MAX_IN_MEMORY_AGENT_AUDIENCES
    ? value
    : Object.fromEntries(entries.slice(-MAX_IN_MEMORY_AGENT_AUDIENCES));
}

function buildSnapshot(): PersistentAgentAudienceSnapshot {
  return { audiences };
}

function emit(): void {
  snapshot = buildSnapshot();
  for (const listener of listeners) listener();
}

export function getPersistentAgentAudienceScope({
  ownerPubkey,
  channelId,
  composerKey,
}: PersistentAgentAudienceScopeInput): string | null {
  const owner = ownerPubkey.trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(owner) || !channelId) return null;
  const composer =
    composerKey?.trim() && composerKey.trim() !== channelId
      ? composerKey.trim()
      : "channel";
  return `${owner}:${channelId}:${composer}`;
}

export function resetPersistentAgentAudienceStore(): void {
  revisionClock += 1;
  defaultRevision = revisionClock;
  revisions.clear();
  excludedPubkeysByScope.clear();
  audiences = {};
  emit();
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
    if (Object.keys(audiences).at(-1) === scope) return;
    const nextAudiences = { ...audiences };
    delete nextAudiences[scope];
    audiences = boundAudiences({ ...nextAudiences, [scope]: current });
    return;
  }

  const nextAudiences = { ...audiences };
  delete nextAudiences[scope];
  audiences = boundAudiences({ ...nextAudiences, [scope]: normalized });
  for (const excludedScope of excludedPubkeysByScope.keys()) {
    if (!Object.hasOwn(audiences, excludedScope)) {
      excludedPubkeysByScope.delete(excludedScope);
    }
  }
  for (const revisedScope of revisions.keys()) {
    if (!Object.hasOwn(audiences, revisedScope)) revisions.delete(revisedScope);
  }
  revisionClock += 1;
  revisions.set(scope, revisionClock);
  emit();
}

export function getPersistentAgentAudienceRevision(scope: string): number {
  return revisions.get(scope) ?? defaultRevision;
}

export function promotePersistentAgentAudienceIfUnchanged({
  expectedRevision,
  reinstateExcluded = false,
  pubkeys,
  scope,
}: {
  expectedRevision: number;
  reinstateExcluded?: boolean;
  pubkeys: Iterable<string>;
  scope: string;
}): { promotedPubkeys: string[]; revision: number } | null {
  if (getPersistentAgentAudienceRevision(scope) !== expectedRevision)
    return null;
  const normalizedPubkeys = normalizePubkeys(pubkeys);
  const promotedPubkeys = normalizedPubkeys.filter(
    (pubkey) =>
      !(audiences[scope] ?? []).includes(pubkey) &&
      (reinstateExcluded || !excludedPubkeysByScope.get(scope)?.has(pubkey)),
  );
  if (promotedPubkeys.length === 0) return null;
  if (reinstateExcluded) {
    const excluded = excludedPubkeysByScope.get(scope);
    for (const pubkey of promotedPubkeys) excluded?.delete(pubkey);
    if (excluded?.size === 0) excludedPubkeysByScope.delete(scope);
  }
  setPersistentAgentAudience(scope, [
    ...(audiences[scope] ?? []),
    ...promotedPubkeys,
  ]);
  return {
    promotedPubkeys,
    revision: getPersistentAgentAudienceRevision(scope),
  };
}

export function removePersistentAgentAudienceMembersIfUnchanged({
  expectedRevision,
  pubkeys,
  scope,
}: {
  expectedRevision: number;
  pubkeys: Iterable<string>;
  scope: string;
}): boolean {
  if (getPersistentAgentAudienceRevision(scope) !== expectedRevision)
    return false;
  const removals = new Set(normalizePubkeys(pubkeys));
  setPersistentAgentAudience(
    scope,
    (audiences[scope] ?? []).filter((pubkey) => !removals.has(pubkey)),
  );
  return true;
}

export function initializePersistentAgentAudience(
  scope: string,
  pubkeys: Iterable<string>,
): void {
  if (!scope) return;
  const excluded = excludedPubkeysByScope.get(scope);
  const initialPubkeys = normalizePubkeys(pubkeys).filter(
    (pubkey) =>
      !(audiences[scope] ?? []).includes(pubkey) && !excluded?.has(pubkey),
  );
  if (initialPubkeys.length === 0) return;
  setPersistentAgentAudience(scope, [
    ...(audiences[scope] ?? []),
    ...initialPubkeys,
  ]);
}

export function addPersistentAgentAudienceMember(
  scope: string,
  pubkey: string,
): void {
  const normalized = normalizePubkeys([pubkey])[0];
  if (!normalized) return;
  excludedPubkeysByScope.get(scope)?.delete(normalized);
  setPersistentAgentAudience(scope, [...(audiences[scope] ?? []), normalized]);
}

export function excludePersistentAgentAudienceMember(
  scope: string,
  pubkey: string,
): void {
  const normalized = normalizePubkeys([pubkey])[0];
  if (!scope || !normalized) return;
  const excluded = excludedPubkeysByScope.get(scope) ?? new Set<string>();
  excluded.add(normalized);
  excludedPubkeysByScope.set(scope, excluded);
  removePersistentAgentAudienceMember(scope, normalized);
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

export function getPersistentAgentAudienceSnapshot(): PersistentAgentAudienceSnapshot {
  return snapshot;
}

function getSnapshot(): PersistentAgentAudienceSnapshot {
  return getPersistentAgentAudienceSnapshot();
}

const serverSnapshot: PersistentAgentAudienceSnapshot = {
  audiences: {},
};

export function usePersistentAgentAudience(scope: string | null): {
  pubkeys: readonly string[];
  addPubkey: (pubkey: string) => void;
  removePubkey: (pubkey: string) => void;
  excludePubkey: (pubkey: string) => void;
  clear: () => void;
} {
  const state = React.useSyncExternalStore(
    subscribe,
    getSnapshot,
    () => serverSnapshot,
  );
  const resolvedScope = scope ?? "";
  return {
    pubkeys: resolvedScope ? (state.audiences[resolvedScope] ?? []) : [],
    addPubkey: React.useCallback(
      (pubkey) => addPersistentAgentAudienceMember(resolvedScope, pubkey),
      [resolvedScope],
    ),
    removePubkey: React.useCallback(
      (pubkey) => removePersistentAgentAudienceMember(resolvedScope, pubkey),
      [resolvedScope],
    ),
    excludePubkey: React.useCallback(
      (pubkey) => excludePersistentAgentAudienceMember(resolvedScope, pubkey),
      [resolvedScope],
    ),
    clear: React.useCallback(
      () => setPersistentAgentAudience(resolvedScope, []),
      [resolvedScope],
    ),
  };
}

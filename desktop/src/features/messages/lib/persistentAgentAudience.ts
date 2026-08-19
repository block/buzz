import * as React from "react";

const AUDIENCES_STORAGE_KEY_PREFIX = "buzz:persistent-agent-audiences:v3";
export const MAX_PERSISTENT_AGENT_AUDIENCES = 200;

const listeners = new Set<() => void>();
let activeCommunityId: string | null = null;
let audiences: Record<string, string[]> = {};
let snapshot = buildSnapshot();

export type PersistentAgentAudienceSnapshot = Readonly<{
  audiences: Readonly<Record<string, readonly string[]>>;
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

function boundAudiences(
  value: Record<string, string[]>,
): Record<string, string[]> {
  const entries = Object.entries(value);
  return entries.length <= MAX_PERSISTENT_AGENT_AUDIENCES
    ? value
    : Object.fromEntries(entries.slice(-MAX_PERSISTENT_AGENT_AUDIENCES));
}

function getAudiencesStorageKey(communityId: string): string {
  return `${AUDIENCES_STORAGE_KEY_PREFIX}:${encodeURIComponent(communityId)}`;
}

function readAudiences(communityId: string): Record<string, string[]> {
  if (typeof window === "undefined") return {};
  try {
    const parsed: unknown = JSON.parse(
      window.localStorage.getItem(getAudiencesStorageKey(communityId)) ?? "{}",
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
    return boundAudiences(result);
  } catch {
    return {};
  }
}

function buildSnapshot(): PersistentAgentAudienceSnapshot {
  return { audiences };
}

function emit(): void {
  snapshot = buildSnapshot();
  for (const listener of listeners) listener();
}

function persistAudiences(): void {
  if (!activeCommunityId) return;
  try {
    window.localStorage.setItem(
      getAudiencesStorageKey(activeCommunityId),
      JSON.stringify(audiences),
    );
  } catch {
    // Persistence is best-effort; the live session still uses in-memory state.
  }
}

export function getPersistentAgentAudienceScope({
  ownerPubkey,
  channelId,
}: PersistentAgentAudienceScopeInput): string | null {
  const owner = ownerPubkey.trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(owner) || !channelId) return null;
  // Thread composers intentionally share their parent channel's audience.
  return `${owner}:${channelId}:channel`;
}

export function initPersistentAgentAudienceStore(communityId: string): void {
  const normalizedCommunityId = communityId.trim();
  if (!normalizedCommunityId) {
    resetPersistentAgentAudienceStore();
    return;
  }
  activeCommunityId = normalizedCommunityId;
  audiences = readAudiences(normalizedCommunityId);
  emit();
}

export function resetPersistentAgentAudienceStore(): void {
  activeCommunityId = null;
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
    persistAudiences();
    return;
  }

  const nextAudiences = { ...audiences };
  delete nextAudiences[scope];
  audiences = boundAudiences({ ...nextAudiences, [scope]: normalized });
  persistAudiences();
  emit();
}

export function addPersistentAgentAudienceMember(
  scope: string,
  pubkey: string,
): void {
  setPersistentAgentAudience(scope, [...(audiences[scope] ?? []), pubkey]);
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
    clear: React.useCallback(
      () => setPersistentAgentAudience(resolvedScope, []),
      [resolvedScope],
    ),
  };
}

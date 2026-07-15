import * as React from "react";

const ENABLED_STORAGE_KEY = "buzz:keep-addressed-agents-active";
const AUDIENCES_STORAGE_KEY = "buzz:persistent-agent-audiences:v1";

const listeners = new Set<() => void>();
let enabled = readEnabled();
let audiences = readAudiences();
let snapshot = buildSnapshot();

export type PersistentAgentAudienceSnapshot = Readonly<{
  enabled: boolean;
  audiences: Readonly<Record<string, readonly string[]>>;
}>;

function normalizePubkeys(pubkeys: Iterable<string>): string[] {
  return [...new Set([...pubkeys].map((pubkey) => pubkey.trim().toLowerCase()))]
    .filter((pubkey) => /^[0-9a-f]{64}$/.test(pubkey))
    .sort();
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
        const normalized = normalizePubkeys(
          value.filter((entry): entry is string => typeof entry === "string"),
        );
        if (normalized.length > 0) result[scope] = normalized;
      }
    }
    return result;
  } catch {
    return {};
  }
}

function buildSnapshot(): PersistentAgentAudienceSnapshot {
  return { enabled, audiences };
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

export function setPersistentAgentAudienceEnabled(nextEnabled: boolean): void {
  if (enabled === nextEnabled) return;
  enabled = nextEnabled;
  if (!nextEnabled && Object.keys(audiences).length > 0) {
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

export function getPersistentAgentAudienceScope(
  channelId: string,
  draftKey: string,
): string {
  return `${channelId}:${draftKey}`;
}

export function setPersistentAgentAudience(
  scope: string,
  pubkeys: Iterable<string>,
): void {
  if (!scope) return;
  const normalized = normalizePubkeys(pubkeys);
  const current = audiences[scope] ?? [];
  if (
    current.length === normalized.length &&
    current.every((pubkey, index) => pubkey === normalized[index])
  ) {
    return;
  }

  const next = { ...audiences };
  if (normalized.length > 0) next[scope] = normalized;
  else delete next[scope];
  audiences = next;
  persistAudiences();
  emit();
}

export function addPersistentAgentAudienceMembers(
  scope: string,
  pubkeys: Iterable<string>,
): void {
  if (!enabled || !scope) return;
  setPersistentAgentAudience(scope, [...(audiences[scope] ?? []), ...pubkeys]);
}

export function addPersistentAgentAudienceMembersForDraft({
  capturedChannelId,
  explicitAgentPubkeys,
  sentDraftKey,
}: {
  capturedChannelId: string | null;
  explicitAgentPubkeys: string[];
  sentDraftKey: string | null | undefined;
}): void {
  if (!enabled || !capturedChannelId || !sentDraftKey) return;
  addPersistentAgentAudienceMembers(
    getPersistentAgentAudienceScope(capturedChannelId, sentDraftKey),
    explicitAgentPubkeys,
  );
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
};

export function usePersistentAgentAudience(scope: string | null): {
  enabled: boolean;
  pubkeys: readonly string[];
  setEnabled: (enabled: boolean) => void;
  addDraftPubkeys: typeof addPersistentAgentAudienceMembersForDraft;
  removePubkey: (pubkey: string) => void;
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
    setEnabled: setPersistentAgentAudienceEnabled,
    addDraftPubkeys: addPersistentAgentAudienceMembersForDraft,
    removePubkey: React.useCallback(
      (pubkey) => removePersistentAgentAudienceMember(resolvedScope, pubkey),
      [resolvedScope],
    ),
  };
}

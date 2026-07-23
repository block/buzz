import * as React from "react";

import type { RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

const STORAGE_PREFIX = "buzz.external-agent-presentations.v1";
const CHANGE_EVENT = "buzz-external-agent-presentations-changed";

export type ExternalAgentPresentation = {
  displayName: string | null;
  avatarUrl: string | null;
};

export type ExternalAgentPresentations = Record<
  string,
  ExternalAgentPresentation
>;

const EMPTY_PRESENTATIONS: ExternalAgentPresentations = Object.freeze({});
let cachedStorageKey = "";
let cachedRawValue: string | null = null;
let cachedPresentations = EMPTY_PRESENTATIONS;

export function externalAgentPresentationScope({
  identityPubkey,
  relayUrl,
}: {
  identityPubkey: string | null | undefined;
  relayUrl: string | null | undefined;
}): string | null {
  const owner = identityPubkey?.trim().toLowerCase();
  const relay = relayUrl?.trim().toLowerCase();
  return owner && relay ? `${owner}:${relay}` : null;
}

function storageKey(scope: string) {
  return `${STORAGE_PREFIX}:${scope}`;
}

function parsePresentations(raw: string | null): ExternalAgentPresentations {
  if (!raw) return EMPTY_PRESENTATIONS;

  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return EMPTY_PRESENTATIONS;
    }

    const presentations: ExternalAgentPresentations = {};
    for (const [pubkey, value] of Object.entries(parsed)) {
      if (!value || typeof value !== "object" || Array.isArray(value)) continue;
      const record = value as Record<string, unknown>;
      presentations[normalizePubkey(pubkey)] = {
        displayName:
          typeof record.displayName === "string"
            ? record.displayName.trim() || null
            : null,
        avatarUrl:
          typeof record.avatarUrl === "string"
            ? record.avatarUrl.trim() || null
            : null,
      };
    }
    return presentations;
  } catch {
    return EMPTY_PRESENTATIONS;
  }
}

export function readExternalAgentPresentations(
  scope: string | null,
): ExternalAgentPresentations {
  if (!scope || typeof window === "undefined") return EMPTY_PRESENTATIONS;

  const key = storageKey(scope);
  const raw = window.localStorage.getItem(key);
  if (key === cachedStorageKey && raw === cachedRawValue) {
    return cachedPresentations;
  }

  cachedStorageKey = key;
  cachedRawValue = raw;
  cachedPresentations = parsePresentations(raw);
  return cachedPresentations;
}

export function saveExternalAgentPresentation(
  scope: string,
  pubkey: string,
  presentation: ExternalAgentPresentation | null,
) {
  if (typeof window === "undefined") return;

  const key = storageKey(scope);
  const current = { ...readExternalAgentPresentations(scope) };
  const normalizedPubkey = normalizePubkey(pubkey);
  if (presentation) {
    current[normalizedPubkey] = {
      displayName: presentation.displayName?.trim() || null,
      avatarUrl: presentation.avatarUrl?.trim() || null,
    };
  } else {
    delete current[normalizedPubkey];
  }

  const raw = Object.keys(current).length > 0 ? JSON.stringify(current) : null;
  if (raw === null) window.localStorage.removeItem(key);
  else window.localStorage.setItem(key, raw);

  cachedStorageKey = key;
  cachedRawValue = raw;
  cachedPresentations =
    Object.keys(current).length > 0 ? current : EMPTY_PRESENTATIONS;
  window.dispatchEvent(new CustomEvent(CHANGE_EVENT, { detail: { scope } }));
}

function subscribe(scope: string | null, onStoreChange: () => void) {
  if (!scope || typeof window === "undefined") return () => {};
  const key = storageKey(scope);
  const handleChange = (event: Event) => {
    if (event instanceof StorageEvent && event.key !== key) return;
    if (
      event instanceof CustomEvent &&
      (event.detail as { scope?: unknown } | null)?.scope !== scope
    ) {
      return;
    }
    cachedStorageKey = "";
    onStoreChange();
  };
  window.addEventListener("storage", handleChange);
  window.addEventListener(CHANGE_EVENT, handleChange);
  return () => {
    window.removeEventListener("storage", handleChange);
    window.removeEventListener(CHANGE_EVENT, handleChange);
  };
}

export function useExternalAgentPresentations(scope: string | null) {
  return React.useSyncExternalStore(
    React.useCallback(
      (onStoreChange) => subscribe(scope, onStoreChange),
      [scope],
    ),
    React.useCallback(() => readExternalAgentPresentations(scope), [scope]),
    () => EMPTY_PRESENTATIONS,
  );
}

export function applyExternalAgentPresentations(
  agents: readonly RelayAgent[],
  presentations: ExternalAgentPresentations,
): RelayAgent[] {
  return agents.map((agent) => {
    const presentation = presentations[normalizePubkey(agent.pubkey)];
    if (!presentation) return agent;
    return {
      ...agent,
      name: presentation.displayName ?? agent.name,
      avatarUrl: presentation.avatarUrl ?? agent.avatarUrl,
    };
  });
}

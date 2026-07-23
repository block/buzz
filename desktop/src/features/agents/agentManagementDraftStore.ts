import * as React from "react";

import type { AgentManagementRequest } from "./agentManagement";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

type Subscriber = () => void;

export type PendingAgentManagementDraft = {
  agentPubkey: string;
  createdAt: string;
  request: AgentManagementRequest;
};

type StoredDrafts = Record<string, PendingAgentManagementDraft>;

const STORE_KEY_PREFIX = "buzz-agent-management-drafts.v1";
const MAX_PENDING_DRAFTS = 100;

const subscribers = new Set<Subscriber>();
const reviewRequestSubscribers = new Set<Subscriber>();
let version = 0;
let reviewRequestVersion = 0;
let currentPubkey = "";
let currentRelayScope = "";
let memCache: Map<string, PendingAgentManagementDraft> | null = null;

function canonicalizeRelayScope(relayUrl: string): string {
  const trimmed = relayUrl.trim().replace(/\/+$/, "");
  if (!trimmed) return "";
  try {
    const u = new URL(trimmed);
    const path = u.pathname.replace(/\/+$/, "");
    return `${u.protocol}//${u.host}${path}${u.search}`;
  } catch {
    return trimmed.toLowerCase();
  }
}

function storageKey(): string {
  return `${STORE_KEY_PREFIX}:${currentRelayScope}:${currentPubkey}`;
}

function notifySubscribers(): void {
  version += 1;
  for (const subscriber of subscribers) {
    subscriber();
  }
}

function subscribeToStore(callback: Subscriber): () => void {
  subscribers.add(callback);
  return () => {
    subscribers.delete(callback);
  };
}

function getStoreSnapshot(): number {
  return version;
}

function notifyReviewRequestSubscribers(): void {
  reviewRequestVersion += 1;
  for (const subscriber of reviewRequestSubscribers) {
    subscriber();
  }
}

function subscribeToReviewRequests(callback: Subscriber): () => void {
  reviewRequestSubscribers.add(callback);
  return () => {
    reviewRequestSubscribers.delete(callback);
  };
}

function getReviewRequestSnapshot(): number {
  return reviewRequestVersion;
}

function isValidDraft(value: unknown): value is PendingAgentManagementDraft {
  if (typeof value !== "object" || value === null) return false;
  const draft = value as Partial<PendingAgentManagementDraft>;
  return (
    typeof draft.agentPubkey === "string" &&
    typeof draft.createdAt === "string" &&
    typeof draft.request === "object" &&
    draft.request !== null &&
    typeof draft.request.requestId === "string" &&
    (draft.request.action === "create" || draft.request.action === "update")
  );
}

function readStore(): Map<string, PendingAgentManagementDraft> {
  if (memCache !== null) return memCache;

  const map = new Map<string, PendingAgentManagementDraft>();
  if (!currentPubkey) {
    memCache = map;
    return map;
  }

  const raw = localStorage.getItem(storageKey());
  if (!raw) {
    memCache = map;
    return map;
  }

  try {
    const parsed: unknown = JSON.parse(raw);
    if (
      parsed !== null &&
      typeof parsed === "object" &&
      !Array.isArray(parsed)
    ) {
      for (const [requestId, value] of Object.entries(parsed as StoredDrafts)) {
        if (isValidDraft(value) && value.request.requestId === requestId) {
          map.set(requestId, value);
        }
      }
    }
  } catch (error) {
    console.debug(
      "[agentManagementDraftStore] localStorage corrupt, starting fresh:",
      error,
    );
  }

  memCache = map;
  return map;
}

function flushStore(map: Map<string, PendingAgentManagementDraft>): boolean {
  if (!currentPubkey) return false;
  const entries = Array.from(map.entries());
  const limitedEntries =
    entries.length > MAX_PENDING_DRAFTS
      ? entries.slice(entries.length - MAX_PENDING_DRAFTS)
      : entries;
  const wrote = setLocalStorageItemWithRecovery(
    storageKey(),
    JSON.stringify(Object.fromEntries(limitedEntries)),
  );
  if (wrote && limitedEntries.length !== entries.length) {
    memCache = new Map(limitedEntries);
  }
  return wrote;
}

export function initAgentManagementDraftStore(
  pubkey: string,
  relayUrl = "",
): void {
  const relayScope = canonicalizeRelayScope(relayUrl);
  if (currentPubkey !== pubkey || currentRelayScope !== relayScope) {
    memCache = null;
  }
  currentPubkey = pubkey;
  currentRelayScope = relayScope;
  readStore();
  notifySubscribers();
}

export function resetAgentManagementDraftStore(): void {
  currentPubkey = "";
  currentRelayScope = "";
  memCache = null;
  notifySubscribers();
}

export function enqueueAgentManagementDraft(
  agentPubkey: string,
  request: AgentManagementRequest,
  now = new Date(),
): boolean {
  const store = readStore();
  if (store.has(request.requestId)) {
    return false;
  }
  store.set(request.requestId, {
    agentPubkey,
    createdAt: now.toISOString(),
    request,
  });
  flushStore(store);
  notifySubscribers();
  return true;
}

export function removeAgentManagementDraft(requestId: string): boolean {
  const store = readStore();
  if (!store.delete(requestId)) {
    return false;
  }
  flushStore(store);
  notifySubscribers();
  return true;
}

export function getPendingAgentManagementDrafts(): PendingAgentManagementDraft[] {
  return Array.from(readStore().values()).sort((left, right) =>
    left.createdAt.localeCompare(right.createdAt),
  );
}

export function peekPendingAgentManagementDraft():
  | PendingAgentManagementDraft
  | undefined {
  return getPendingAgentManagementDrafts()[0];
}

export function getPendingAgentManagementDraftCount(): number {
  return readStore().size;
}

export function requestAgentManagementDraftReview(): void {
  notifyReviewRequestSubscribers();
}

export function useAgentManagementDraftCount(): number {
  React.useSyncExternalStore(subscribeToStore, getStoreSnapshot);
  return getPendingAgentManagementDraftCount();
}

export function useAgentManagementReviewRequestVersion(): number {
  return React.useSyncExternalStore(
    subscribeToReviewRequests,
    getReviewRequestSnapshot,
  );
}

export const _testAgentManagementDraftStore = {
  subscribeToStore,
};

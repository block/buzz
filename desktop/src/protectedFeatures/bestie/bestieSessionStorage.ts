import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";

export const BESTIE_SESSION_STORAGE_PREFIX = "buzz-bestie-session.v1";

export type BestieSessionBoundary = {
  /** Message ids present before this session started. */
  baselineMessageIds: string[];
  /** created_at of the first message sent in this session. */
  firstMessageCreatedAt: number;
  /** Id of the first message sent in this session (session root). */
  sessionRootId: string;
};

export type BestieSessionScope = {
  agentPubkey: string;
  ownerPubkey: string;
  relayUrl: string;
};

export function bestieSessionStorageKey(scope: BestieSessionScope): string {
  const relay =
    canonicalRelayUrl(scope.relayUrl) ?? scope.relayUrl.trim().toLowerCase();
  return [
    BESTIE_SESSION_STORAGE_PREFIX,
    relay,
    scope.ownerPubkey.toLowerCase(),
    scope.agentPubkey.toLowerCase(),
  ].join(":");
}

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

export function parseBestieSessionBoundary(
  value: unknown,
): BestieSessionBoundary | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (
    !isFiniteNonNegative(record.firstMessageCreatedAt) ||
    typeof record.sessionRootId !== "string" ||
    record.sessionRootId.length === 0 ||
    !Array.isArray(record.baselineMessageIds)
  ) {
    return null;
  }
  const baselineMessageIds = record.baselineMessageIds.filter(
    (id): id is string => typeof id === "string" && id.length > 0,
  );
  return {
    baselineMessageIds,
    firstMessageCreatedAt: record.firstMessageCreatedAt,
    sessionRootId: record.sessionRootId,
  };
}

export function readBestieSessionBoundary(
  scope: BestieSessionScope,
): BestieSessionBoundary | null {
  try {
    const raw = window.localStorage.getItem(bestieSessionStorageKey(scope));
    if (!raw) return null;
    return parseBestieSessionBoundary(JSON.parse(raw));
  } catch {
    return null;
  }
}

export function writeBestieSessionBoundary(
  scope: BestieSessionScope,
  boundary: BestieSessionBoundary,
): void {
  try {
    window.localStorage.setItem(
      bestieSessionStorageKey(scope),
      JSON.stringify(boundary),
    );
  } catch {
    // Ignore quota / private-mode failures; in-memory session still works.
  }
}

export function clearBestieSessionBoundary(scope: BestieSessionScope): void {
  try {
    window.localStorage.removeItem(bestieSessionStorageKey(scope));
  } catch {
    // Ignore storage failures.
  }
}

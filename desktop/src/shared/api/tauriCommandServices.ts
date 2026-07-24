import { invokeTauri } from "@/shared/api/tauri";

export type MemoryServiceStatus = "ready" | "not_configured" | "unavailable";

export type MemoryServiceReadiness = {
  status: MemoryServiceStatus;
  nodeId: string | null;
  revisionCount: number;
  conflictCount: number;
  endpoint: string | null;
  syncIntervalMinutes: number | null;
  toolAllowlist: string[];
  observedAt: string;
  error: string | null;
};

export type MemoryReplicationResult = {
  status: "ok";
  operation: "pull" | "push";
  sourceNodeId: string;
  targetNodeId: string;
  fromCursor: number;
  toCursor: number;
  accepted: number;
  duplicates: number;
  conflicts: number;
  objects: number;
  tombstones: number;
  pages: number;
  targetConflictCount: number;
  lastSuccess: string;
};

export type PinnedHostEvidence = {
  hostAlias: string;
  fingerprint: string;
  keyType: string;
};

export type MemorySyncResponse = {
  status: "ok" | "error";
  pull: MemoryReplicationResult | null;
  push: MemoryReplicationResult | null;
  pinnedHost: PinnedHostEvidence | null;
  lastSuccess: string | null;
  error: string | null;
};

/**
 * Read the authenticated Mac-local Memory service state. Configuration and
 * credentials are resolved only inside trusted Rust and are never arguments.
 */
export function getMemoryServiceReadiness(): Promise<MemoryServiceReadiness> {
  return invokeTauri<MemoryServiceReadiness>("get_memory_service_readiness");
}

/**
 * Explicitly run one pull and one push through Buzz's pinned SSH tunnel.
 * The renderer cannot choose endpoints, binaries, credentials, or paths.
 */
export function syncMemoryService(): Promise<MemorySyncResponse> {
  return invokeTauri<MemorySyncResponse>("sync_memory_service");
}

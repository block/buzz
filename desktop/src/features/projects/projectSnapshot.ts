import type { QueryClient } from "@tanstack/react-query";

import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";
import type { Project } from "./projectModels";

const STORAGE_KEY_PREFIX = "buzz-projects.v1";
export const PROJECTS_QUERY_KEY = ["projects"] as const;

type ProjectSnapshotScope = {
  pubkey: string;
  relayUrl: string;
};

type StoredProjectSnapshot = {
  integrity: string;
  ownerPubkey: string;
  projects: Project[];
  updatedAt: number;
  version: 1;
};

const snapshotScopes = new WeakMap<QueryClient, ProjectSnapshotScope>();

/**
 * Snapshot hydration uses timestamp zero; only a completed relay enumeration
 * may authorize side effects or suppress the scoped startup lookup.
 */
export function isAuthoritativeProjectData(dataUpdatedAt: number): boolean {
  return dataUpdatedAt > 0;
}

/** Keeps the active-channel fast path live while only a snapshot is present. */
export function shouldUseScopedProjectHomeLookup({
  dataUpdatedAt,
  hasEnumeratedProjectHome,
  isHuddleTranscript,
}: {
  dataUpdatedAt: number;
  hasEnumeratedProjectHome: boolean;
  isHuddleTranscript: boolean;
}): boolean {
  return (
    !isHuddleTranscript &&
    !hasEnumeratedProjectHome &&
    !isAuthoritativeProjectData(dataUpdatedAt)
  );
}

function projectSnapshotRelayPrefix(relayUrl: string): string {
  return `${STORAGE_KEY_PREFIX}:${normalizeRelayUrl(relayUrl)}:`;
}

export function projectSnapshotKey(
  relayUrl: string,
  ownerPubkey: string,
): string {
  return `${projectSnapshotRelayPrefix(relayUrl)}${ownerPubkey.toLowerCase()}`;
}

function snapshotIntegrity(ownerPubkey: string, projects: Project[]): string {
  const value = JSON.stringify([ownerPubkey.toLowerCase(), projects]);
  let result = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    result ^= value.charCodeAt(index);
    result = Math.imul(result, 0x01000193);
  }
  return (result >>> 0).toString(16).padStart(8, "0");
}

function isProject(value: unknown): value is Project {
  if (typeof value !== "object" || value === null) return false;
  const project = value as Partial<Project>;
  return (
    typeof project.id === "string" &&
    typeof project.owner === "string" &&
    typeof project.projectAddress === "string" &&
    (typeof project.projectChannelId === "string" ||
      project.projectChannelId === null) &&
    Array.isArray(project.repositoryAddresses) &&
    Array.isArray(project.repositories) &&
    typeof project.legacy === "boolean"
  );
}

function parseProjectSnapshot(
  value: unknown,
  ownerPubkey: string,
): Project[] | null {
  if (typeof value !== "object" || value === null) return null;
  const snapshot = value as Partial<StoredProjectSnapshot>;
  if (
    snapshot.version !== 1 ||
    snapshot.ownerPubkey?.toLowerCase() !== ownerPubkey.toLowerCase() ||
    !Array.isArray(snapshot.projects) ||
    !snapshot.projects.every(isProject) ||
    typeof snapshot.integrity !== "string" ||
    snapshot.integrity !== snapshotIntegrity(ownerPubkey, snapshot.projects)
  ) {
    return null;
  }
  return snapshot.projects;
}

export function readProjectSnapshot(
  relayUrl: string,
  ownerPubkey: string,
): Project[] | null {
  try {
    const raw = window.localStorage.getItem(
      projectSnapshotKey(relayUrl, ownerPubkey),
    );
    return raw ? parseProjectSnapshot(JSON.parse(raw), ownerPubkey) : null;
  } catch {
    return null;
  }
}

/**
 * Seeds the last fully validated project collection into a community's fresh
 * query client. Timestamp zero keeps it stale so the relay revalidates it.
 */
export function seedProjectSnapshot(
  queryClient: QueryClient,
  scope: ProjectSnapshotScope,
): void {
  snapshotScopes.set(queryClient, scope);
  const projects = readProjectSnapshot(scope.relayUrl, scope.pubkey);
  if (projects) {
    queryClient.setQueryData(PROJECTS_QUERY_KEY, projects, { updatedAt: 0 });
  }
}

/** Persists a successful complete enumeration for the current community. */
export function persistProjectSnapshot(
  queryClient: QueryClient,
  projects: Project[],
): void {
  const scope = snapshotScopes.get(queryClient);
  if (!scope) return;
  try {
    const snapshot: StoredProjectSnapshot = {
      integrity: snapshotIntegrity(scope.pubkey, projects),
      ownerPubkey: scope.pubkey.toLowerCase(),
      projects,
      updatedAt: Date.now(),
      version: 1,
    };
    setLocalStorageItemWithRecovery(
      projectSnapshotKey(scope.relayUrl, scope.pubkey),
      JSON.stringify(snapshot),
    );
  } catch {
    // Snapshot persistence is optional; live relay data remains authoritative.
  }
}

/** Removes every identity's project snapshot for a deleted community. */
export function removeProjectSnapshotForRelay(relayUrl: string): void {
  try {
    const prefix = projectSnapshotRelayPrefix(relayUrl);
    const keys: string[] = [];
    for (let index = 0; index < window.localStorage.length; index += 1) {
      const key = window.localStorage.key(index);
      if (key?.startsWith(prefix)) keys.push(key);
    }
    for (const key of keys) window.localStorage.removeItem(key);
  } catch {
    // Storage access failures are non-fatal.
  }
}

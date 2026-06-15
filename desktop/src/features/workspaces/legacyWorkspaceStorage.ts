import { invokeTauri } from "@/shared/api/tauri";

const BUZZ_WORKSPACES_KEY = "buzz-workspaces";
const BUZZ_ACTIVE_WORKSPACE_KEY = "buzz-active-workspace-id";
const BUZZ_ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX =
  "buzz-onboarding-complete.v1:";
const LOCAL_DEV_RELAY_URLS = new Set([
  "ws://localhost:3000",
  "ws://127.0.0.1:3000",
]);

type LegacyWorkspaceStorageSnapshot = {
  workspaces: string | null;
  activeWorkspaceId: string | null;
  onboardingCompletions: Array<{
    pubkey: string;
    value: string;
  }>;
};

type StoredWorkspace = {
  relayUrl?: unknown;
};

function parseWorkspaceList(raw: string | null): StoredWorkspace[] | null {
  if (!raw) {
    return null;
  }

  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as StoredWorkspace[]) : null;
  } catch {
    return null;
  }
}

function normalizeRelayUrl(relayUrl: string) {
  return relayUrl.trim().replace(/\/$/, "");
}

function hasOnlyLocalDevWorkspace(raw: string | null): boolean {
  const workspaces = parseWorkspaceList(raw);
  return (
    workspaces?.length === 1 &&
    typeof workspaces[0]?.relayUrl === "string" &&
    LOCAL_DEV_RELAY_URLS.has(normalizeRelayUrl(workspaces[0].relayUrl))
  );
}

function hasNonLocalCurrentWorkspaces(raw: string | null): boolean {
  const workspaces = parseWorkspaceList(raw);
  return (
    workspaces !== null &&
    workspaces.length > 0 &&
    !hasOnlyLocalDevWorkspace(raw)
  );
}

function shouldWriteLegacyWorkspaces({
  currentWorkspacesRaw,
  legacyWorkspacesRaw,
}: {
  currentWorkspacesRaw: string | null;
  legacyWorkspacesRaw: string | null;
}) {
  const legacyWorkspaces = parseWorkspaceList(legacyWorkspacesRaw);
  if (!legacyWorkspaces || legacyWorkspaces.length === 0) {
    return false;
  }

  return !hasNonLocalCurrentWorkspaces(currentWorkspacesRaw);
}

export function applyLegacyWorkspaceStorage(
  legacyStorage: LegacyWorkspaceStorageSnapshot,
  storage: Storage = window.localStorage,
): void {
  const currentWorkspacesRaw = storage.getItem(BUZZ_WORKSPACES_KEY);
  const shouldWriteWorkspaces = shouldWriteLegacyWorkspaces({
    currentWorkspacesRaw,
    legacyWorkspacesRaw: legacyStorage.workspaces,
  });

  if (shouldWriteWorkspaces && legacyStorage.workspaces) {
    storage.setItem(BUZZ_WORKSPACES_KEY, legacyStorage.workspaces);
  }

  const currentActiveWorkspaceId = storage.getItem(BUZZ_ACTIVE_WORKSPACE_KEY);
  if (
    legacyStorage.activeWorkspaceId &&
    (!currentActiveWorkspaceId || shouldWriteWorkspaces)
  ) {
    storage.setItem(BUZZ_ACTIVE_WORKSPACE_KEY, legacyStorage.activeWorkspaceId);
  }

  for (const completion of legacyStorage.onboardingCompletions) {
    const key = `${BUZZ_ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX}${completion.pubkey}`;
    if (storage.getItem(key) === null) {
      storage.setItem(key, completion.value);
    }
  }
}

/**
 * Seed Buzz localStorage from legacy Sprout WebKit localStorage before the app
 * renders providers that read workspace state. The native command reads the old
 * app identifier's WebKit SQLite database; this frontend step writes only when
 * Buzz does not already have workspace state, except for the known broken
 * Sprout→Buzz first-run handoff that created a single localhost workspace.
 */
export async function migrateLegacyWorkspaceStorageBeforeRender(): Promise<void> {
  if (typeof window === "undefined") {
    return;
  }

  const currentWorkspacesRaw = window.localStorage.getItem(BUZZ_WORKSPACES_KEY);
  const hasCurrentActiveWorkspace = window.localStorage.getItem(
    BUZZ_ACTIVE_WORKSPACE_KEY,
  );
  if (
    currentWorkspacesRaw &&
    hasCurrentActiveWorkspace &&
    !hasOnlyLocalDevWorkspace(currentWorkspacesRaw)
  ) {
    return;
  }

  try {
    applyLegacyWorkspaceStorage(
      await invokeTauri<LegacyWorkspaceStorageSnapshot>(
        "get_legacy_workspace_storage",
      ),
    );
  } catch (error) {
    console.warn("Failed to read legacy Sprout workspace storage.", error);
  }
}

/**
 * The active Documents vault path.
 *
 * Deliberately **global and per-machine**, not per-community: notes are the
 * user's own, and switching communities must not swap their vault. That is also
 * why this is not wired into `resetCommunityState()` — see the Documents notes
 * in the feature plan.
 *
 * The string here is only a hint for the UI and for boot reconciliation. Access
 * is granted by the Rust side (`set_active_vault`), which holds the real root;
 * writing this key grants nothing on its own.
 */
import * as React from "react";

export const VAULT_PATH_KEY = "buzz.documents.vaultPath.v1";

type Listener = () => void;
const listeners = new Set<Listener>();

function readStoredVaultPath(): string | null {
  try {
    const raw = window.localStorage.getItem(VAULT_PATH_KEY);
    const trimmed = raw?.trim();
    return trimmed ? trimmed : null;
  } catch {
    return null;
  }
}

function emitChange(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);

  // Cross-window sync, mirroring shared/features/useFeatureEnabled.ts.
  const handleStorage = (event: StorageEvent) => {
    if (event.key === VAULT_PATH_KEY) {
      emitChange();
    }
  };
  window.addEventListener("storage", handleStorage);

  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", handleStorage);
  };
}

// useSyncExternalStore needs a referentially stable snapshot. The stored value
// is already a string (or null), so it is stable by construction — no caching
// dance is needed here, unlike the feature-overrides object.
function getSnapshot(): string | null {
  return readStoredVaultPath();
}

function getServerSnapshot(): string | null {
  return null;
}

/** Persist the chosen vault path and notify every subscriber in this window. */
export function storeVaultPath(path: string | null): void {
  try {
    if (path) {
      window.localStorage.setItem(VAULT_PATH_KEY, path);
    } else {
      window.localStorage.removeItem(VAULT_PATH_KEY);
    }
  } catch {
    // Keep the in-memory value usable even when storage is unavailable.
  }
  emitChange();
}

/** The active vault path, or `null` when none has been chosen. */
export function useVaultPath(): string | null {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

import * as React from "react";

export const ACP_RUNTIME_VISIBILITY_STORAGE_KEY =
  "buzz-agent-runtime-visibility.v1";

type RuntimeVisibilityStorage = Pick<Storage, "getItem" | "setItem">;

const EMPTY_DISABLED_RUNTIME_IDS: readonly string[] = Object.freeze([]);
const listeners = new Set<() => void>();

let cachedSerializedValue: string | null | undefined;
let cachedDisabledRuntimeIds = EMPTY_DISABLED_RUNTIME_IDS;

function normalizeRuntimeId(runtimeId: string): string {
  return runtimeId.trim().toLowerCase();
}

function getLocalStorage(): RuntimeVisibilityStorage | null {
  if (typeof window === "undefined") return null;

  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function parseDisabledAcpRuntimeIds(
  serialized: string | null,
): readonly string[] {
  if (!serialized) return EMPTY_DISABLED_RUNTIME_IDS;

  try {
    const parsed: unknown = JSON.parse(serialized);
    if (!Array.isArray(parsed)) return EMPTY_DISABLED_RUNTIME_IDS;

    const runtimeIds = [
      ...new Set(
        parsed
          .filter((value): value is string => typeof value === "string")
          .map(normalizeRuntimeId)
          .filter(Boolean),
      ),
    ].sort();
    return runtimeIds.length > 0
      ? Object.freeze(runtimeIds)
      : EMPTY_DISABLED_RUNTIME_IDS;
  } catch {
    return EMPTY_DISABLED_RUNTIME_IDS;
  }
}

export function readDisabledAcpRuntimeIds(
  storage: Pick<RuntimeVisibilityStorage, "getItem">,
): readonly string[] {
  try {
    return parseDisabledAcpRuntimeIds(
      storage.getItem(ACP_RUNTIME_VISIBILITY_STORAGE_KEY),
    );
  } catch {
    return EMPTY_DISABLED_RUNTIME_IDS;
  }
}

export function nextDisabledAcpRuntimeIds(
  current: readonly string[],
  runtimeId: string,
  enabled: boolean,
): readonly string[] {
  const normalizedRuntimeId = normalizeRuntimeId(runtimeId);
  if (!normalizedRuntimeId) return current;

  const next = new Set(current.map(normalizeRuntimeId).filter(Boolean));
  if (enabled) {
    next.delete(normalizedRuntimeId);
  } else {
    next.add(normalizedRuntimeId);
  }

  const runtimeIds = [...next].sort();
  return runtimeIds.length > 0
    ? Object.freeze(runtimeIds)
    : EMPTY_DISABLED_RUNTIME_IDS;
}

export function filterEnabledAcpRuntimes<T extends { id: string }>(
  runtimes: readonly T[],
  disabledRuntimeIds: readonly string[],
): T[] {
  if (disabledRuntimeIds.length === 0) return [...runtimes];

  const disabled = new Set(disabledRuntimeIds.map(normalizeRuntimeId));
  return runtimes.filter(
    (runtime) => !disabled.has(normalizeRuntimeId(runtime.id)),
  );
}

/**
 * Prevent a disabled runtime from remaining the effective global preference.
 *
 * The persisted config is left untouched until the user next saves defaults;
 * consumers immediately fall back through the normal runtime selection path.
 */
export function maskDisabledAcpRuntimePreference<
  T extends { preferred_runtime: string | null },
>(config: T, disabledRuntimeIds: readonly string[]): T {
  const preferredRuntime = config.preferred_runtime;
  if (
    !preferredRuntime ||
    !disabledRuntimeIds
      .map(normalizeRuntimeId)
      .includes(normalizeRuntimeId(preferredRuntime))
  ) {
    return config;
  }

  return { ...config, preferred_runtime: null };
}

function getDisabledRuntimeIdsSnapshot(): readonly string[] {
  const storage = getLocalStorage();
  if (!storage) return EMPTY_DISABLED_RUNTIME_IDS;

  let serialized: string | null;
  try {
    serialized = storage.getItem(ACP_RUNTIME_VISIBILITY_STORAGE_KEY);
  } catch {
    return EMPTY_DISABLED_RUNTIME_IDS;
  }

  if (serialized === cachedSerializedValue) {
    return cachedDisabledRuntimeIds;
  }

  cachedSerializedValue = serialized;
  cachedDisabledRuntimeIds = parseDisabledAcpRuntimeIds(serialized);
  return cachedDisabledRuntimeIds;
}

function subscribeToRuntimeVisibility(onStoreChange: () => void): () => void {
  listeners.add(onStoreChange);

  const handleStorage = (event: StorageEvent) => {
    if (
      event.key !== null &&
      event.key !== ACP_RUNTIME_VISIBILITY_STORAGE_KEY
    ) {
      return;
    }
    cachedSerializedValue = undefined;
    onStoreChange();
  };
  window.addEventListener("storage", handleStorage);

  return () => {
    listeners.delete(onStoreChange);
    window.removeEventListener("storage", handleStorage);
  };
}

export function setAcpRuntimeEnabled(
  runtimeId: string,
  enabled: boolean,
): boolean {
  const storage = getLocalStorage();
  if (!storage) return false;

  const current = getDisabledRuntimeIdsSnapshot();
  const next = nextDisabledAcpRuntimeIds(current, runtimeId, enabled);
  if (next === current) return true;

  const serialized = JSON.stringify(next);
  try {
    storage.setItem(ACP_RUNTIME_VISIBILITY_STORAGE_KEY, serialized);
  } catch {
    return false;
  }

  cachedSerializedValue = serialized;
  cachedDisabledRuntimeIds = next;
  for (const listener of listeners) listener();
  return true;
}

export function useDisabledAcpRuntimeIds(): readonly string[] {
  return React.useSyncExternalStore(
    subscribeToRuntimeVisibility,
    getDisabledRuntimeIdsSnapshot,
    () => EMPTY_DISABLED_RUNTIME_IDS,
  );
}

export function useSelectableAcpRuntimes<T extends { id: string }>(
  runtimes: readonly T[],
): T[] {
  const disabledRuntimeIds = useDisabledAcpRuntimeIds();
  return React.useMemo(
    () => filterEnabledAcpRuntimes(runtimes, disabledRuntimeIds),
    [disabledRuntimeIds, runtimes],
  );
}

export function useAcpRuntimeEnabled(runtimeId: string): boolean {
  const disabledRuntimeIds = useDisabledAcpRuntimeIds();
  const normalizedRuntimeId = normalizeRuntimeId(runtimeId);
  return !disabledRuntimeIds.includes(normalizedRuntimeId);
}

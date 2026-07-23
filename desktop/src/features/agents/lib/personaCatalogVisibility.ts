import type { SnapshotMemoryLevel } from "@/shared/api/tauriPersonas";

const PERSONA_CATALOG_VISIBILITY_STORAGE_KEY =
  "buzz-persona-catalog-visibility-v1";
const PERSONA_CATALOG_PUBLISHED_VERSIONS_STORAGE_KEY =
  "buzz-persona-catalog-published-versions-v1";
const PERSONA_CATALOG_MEMORY_LEVELS_STORAGE_KEY =
  "buzz-persona-catalog-memory-levels-v1";

export type PublishedCatalogPersonaVersions = Record<string, string>;
export type CatalogPersonaMemoryLevels = Record<string, SnapshotMemoryLevel>;
export type CatalogPersonaShareLevel = "not-shared" | SnapshotMemoryLevel;

const SNAPSHOT_MEMORY_LEVELS = new Set<SnapshotMemoryLevel>([
  "none",
  "core",
  "everything",
]);

function resolveStorage<T extends "getItem" | "setItem">(
  storage: Pick<Storage, T> | null | undefined,
): Pick<Storage, T> | null {
  if (storage !== undefined) return storage;
  if (typeof window === "undefined") return null;

  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

export function readSharedCatalogPersonaIds(
  storage?: Pick<Storage, "getItem"> | null,
): string[] {
  const targetStorage = resolveStorage(storage);
  if (!targetStorage) return [];

  try {
    const raw = targetStorage.getItem(PERSONA_CATALOG_VISIBILITY_STORAGE_KEY);
    if (!raw) return [];

    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];

    return parsed.filter((id): id is string => typeof id === "string");
  } catch {
    return [];
  }
}

export function writeSharedCatalogPersonaIds(
  ids: readonly string[],
  storage?: Pick<Storage, "setItem"> | null,
): void {
  const targetStorage = resolveStorage(storage);
  if (!targetStorage) return;

  try {
    targetStorage.setItem(
      PERSONA_CATALOG_VISIBILITY_STORAGE_KEY,
      JSON.stringify(ids),
    );
  } catch {
    // Catalog visibility is a convenience setting and should not block sharing.
  }
}

export function readCatalogPersonaMemoryLevels(
  storage?: Pick<Storage, "getItem"> | null,
): CatalogPersonaMemoryLevels {
  const targetStorage = resolveStorage(storage);
  if (!targetStorage) return {};

  try {
    const raw = targetStorage.getItem(
      PERSONA_CATALOG_MEMORY_LEVELS_STORAGE_KEY,
    );
    if (!raw) return {};

    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }

    return Object.fromEntries(
      Object.entries(parsed).filter(
        (entry): entry is [string, SnapshotMemoryLevel] =>
          typeof entry[1] === "string" &&
          SNAPSHOT_MEMORY_LEVELS.has(entry[1] as SnapshotMemoryLevel),
      ),
    );
  } catch {
    return {};
  }
}

export function writeCatalogPersonaMemoryLevels(
  levels: Readonly<CatalogPersonaMemoryLevels>,
  storage?: Pick<Storage, "setItem"> | null,
): void {
  const targetStorage = resolveStorage(storage);
  if (!targetStorage) return;

  try {
    targetStorage.setItem(
      PERSONA_CATALOG_MEMORY_LEVELS_STORAGE_KEY,
      JSON.stringify(levels),
    );
  } catch {
    // Catalog publication state should not block sharing.
  }
}

export function readPublishedCatalogPersonaVersions(
  storage?: Pick<Storage, "getItem"> | null,
): PublishedCatalogPersonaVersions {
  const targetStorage = resolveStorage(storage);
  if (!targetStorage) return {};

  try {
    const raw = targetStorage.getItem(
      PERSONA_CATALOG_PUBLISHED_VERSIONS_STORAGE_KEY,
    );
    if (!raw) return {};

    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }

    return Object.fromEntries(
      Object.entries(parsed).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );
  } catch {
    return {};
  }
}

export function writePublishedCatalogPersonaVersions(
  versions: Readonly<PublishedCatalogPersonaVersions>,
  storage?: Pick<Storage, "setItem"> | null,
): void {
  const targetStorage = resolveStorage(storage);
  if (!targetStorage) return;

  try {
    targetStorage.setItem(
      PERSONA_CATALOG_PUBLISHED_VERSIONS_STORAGE_KEY,
      JSON.stringify(versions),
    );
  } catch {
    // Catalog publication state should not block sharing.
  }
}

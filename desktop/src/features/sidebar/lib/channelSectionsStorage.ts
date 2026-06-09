const STORAGE_KEY_PREFIX = "sprout-channel-sections.v1";

export type ChannelSection = {
  id: string;
  name: string;
  order: number;
};

export type ChannelSectionStore = {
  version: 1;
  sections: ChannelSection[];
  assignments: Record<string, string>;
};

export const DEFAULT_STORE: ChannelSectionStore = Object.freeze({
  version: 1,
  sections: [],
  assignments: {},
});

export function storageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

export function stripOrphanedAssignments(
  store: ChannelSectionStore,
): ChannelSectionStore {
  const sectionIds = new Set(store.sections.map((s) => s.id));
  const cleaned = Object.fromEntries(
    Object.entries(store.assignments).filter(([, sid]) => sectionIds.has(sid)),
  );
  if (Object.keys(cleaned).length === Object.keys(store.assignments).length)
    return store;
  return { ...store, assignments: cleaned };
}

export function parseChannelSectionPayload(
  json: unknown,
): ChannelSectionStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  const sections: ChannelSection[] = Array.isArray(obj.sections)
    ? obj.sections.filter(
        (entry: unknown): entry is ChannelSection =>
          typeof entry === "object" &&
          entry !== null &&
          typeof (entry as Record<string, unknown>).id === "string" &&
          typeof (entry as Record<string, unknown>).name === "string" &&
          typeof (entry as Record<string, unknown>).order === "number",
      )
    : [];
  const assignments: Record<string, string> =
    typeof obj.assignments === "object" &&
    obj.assignments !== null &&
    !Array.isArray(obj.assignments)
      ? Object.fromEntries(
          Object.entries(obj.assignments as Record<string, unknown>).filter(
            (entry): entry is [string, string] => typeof entry[1] === "string",
          ),
        )
      : {};
  return stripOrphanedAssignments({ version: 1, sections, assignments });
}

export function readChannelSectionsStore(pubkey: string): ChannelSectionStore {
  try {
    const raw = window.localStorage.getItem(storageKey(pubkey));
    if (!raw) {
      return DEFAULT_STORE;
    }
    const parsed = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null || parsed.version !== 1) {
      return DEFAULT_STORE;
    }
    return parseChannelSectionPayload(parsed) ?? DEFAULT_STORE;
  } catch {
    return DEFAULT_STORE;
  }
}

export function writeChannelSectionsStore(
  pubkey: string,
  store: ChannelSectionStore,
): boolean {
  try {
    window.localStorage.setItem(storageKey(pubkey), JSON.stringify(store));
    return true;
  } catch {
    return false;
  }
}

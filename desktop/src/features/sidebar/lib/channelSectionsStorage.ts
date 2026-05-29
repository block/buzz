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

export const DEFAULT_STORE: ChannelSectionStore = {
  version: 1,
  sections: [],
  assignments: {},
};

function storageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
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
    const sections: ChannelSection[] = Array.isArray(parsed.sections)
      ? parsed.sections.filter(
          (entry: unknown): entry is ChannelSection =>
            typeof entry === "object" &&
            entry !== null &&
            typeof (entry as Record<string, unknown>).id === "string" &&
            typeof (entry as Record<string, unknown>).name === "string" &&
            typeof (entry as Record<string, unknown>).order === "number",
        )
      : [];
    const assignments: Record<string, string> =
      typeof parsed.assignments === "object" &&
      parsed.assignments !== null &&
      !Array.isArray(parsed.assignments)
        ? Object.fromEntries(
            Object.entries(parsed.assignments).filter(
              (entry): entry is [string, string] =>
                typeof entry[1] === "string",
            ),
          )
        : {};
    return { version: 1, sections, assignments };
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

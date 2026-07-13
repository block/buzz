import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";

const STORAGE_KEY_PREFIX = "buzz-channel-sections.v1";

export type ChannelSection = {
  id: string;
  name: string;
  icon?: string;
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

/**
 * Returns the localStorage key for channel sections.
 *
 * When `relayUrl` is provided the key is scoped to that relay (normalized via
 * the same `normalizeRelayUrl` used by all relay-scoped local stores) so
 * sections from different workspaces/relays don't bleed across each other.
 * When omitted the legacy pubkey-only key is returned (used only during
 * one-time migration in `readChannelSectionsStore`).
 */
export function storageKey(pubkey: string, relayUrl?: string): string {
  if (!relayUrl) return `${STORAGE_KEY_PREFIX}:${pubkey}`;
  const normalized = normalizeRelayUrl(relayUrl);
  // Encode the normalized relay so it can't contain the `:` delimiter.
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalized)}`;
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
    ? obj.sections.flatMap((entry: unknown): ChannelSection[] => {
        if (typeof entry !== "object" || entry === null) return [];
        const section = entry as Record<string, unknown>;
        if (
          typeof section.id !== "string" ||
          typeof section.name !== "string" ||
          typeof section.order !== "number"
        ) {
          return [];
        }
        const icon =
          typeof section.icon === "string" && section.icon.trim().length > 0
            ? section.icon.trim()
            : undefined;
        return [
          {
            id: section.id,
            name: section.name,
            ...(icon ? { icon } : {}),
            order: section.order,
          },
        ];
      })
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

function parseRaw(raw: string | null): ChannelSectionStore | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null || parsed.version !== 1) {
      return null;
    }
    return parseChannelSectionPayload(parsed);
  } catch {
    return null;
  }
}

/**
 * Read the section store for `pubkey` scoped to `relayUrl`.
 *
 * On first access for a scoped key, migrates any existing data from the
 * legacy pubkey-only key so users don't lose their sections on upgrade.
 * After a successful migration write the legacy key is deleted, making
 * this a globally one-time operation — subsequent empty scoped-key reads
 * (i.e. a different relay) won't see legacy data and won't trigger
 * cross-relay contamination via the seed-publish path.
 */
export function readChannelSectionsStore(
  pubkey: string,
  relayUrl?: string,
): ChannelSectionStore {
  try {
    const key = storageKey(pubkey, relayUrl);
    const raw = window.localStorage.getItem(key);

    // Scoped key already has data — use it directly.
    if (raw !== null) {
      return parseRaw(raw) ?? DEFAULT_STORE;
    }

    // No scoped data yet.  If we were given a relay scope, attempt a
    // one-time migration from the legacy pubkey-only key.
    if (relayUrl) {
      const legacyKey = storageKey(pubkey);
      const legacyRaw = window.localStorage.getItem(legacyKey);
      const migrated = parseRaw(legacyRaw);
      if (migrated && migrated.sections.length > 0) {
        // Persist under the scoped key and remove the legacy key so this
        // migration cannot fire again for any other relay scope.
        try {
          window.localStorage.setItem(key, JSON.stringify(migrated));
          window.localStorage.removeItem(legacyKey);
        } catch {
          // Ignore write failures — we still return the migrated value.
        }
        return migrated;
      }
    }

    return DEFAULT_STORE;
  } catch {
    return DEFAULT_STORE;
  }
}

export function writeChannelSectionsStore(
  pubkey: string,
  store: ChannelSectionStore,
  relayUrl?: string,
): boolean {
  try {
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify(store),
    );
    return true;
  } catch {
    return false;
  }
}

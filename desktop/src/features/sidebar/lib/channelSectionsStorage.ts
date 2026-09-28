import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

const STORAGE_KEY_PREFIX = "buzz-channel-sections.v1";
export const MAX_CHANNEL_SECTIONS = 100;
export const MAX_CHANNEL_SECTION_ASSIGNMENTS = 1_000;

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

/**
 * Returns the localStorage key for channel sections.
 *
 * When `relayUrl` is provided the key is scoped to that relay (normalized via
 * the same `normalizeRelayUrl` used by all relay-scoped local stores) so
 * sections from different communities/relays don't bleed across each other.
 * When omitted the legacy pubkey-only key is returned (migrated once on the
 * first scoped read).
 */
export function storageKey(pubkey: string, relayUrl?: string): string {
  if (!relayUrl) return `${STORAGE_KEY_PREFIX}:${pubkey}`;
  const normalized = normalizeRelayUrl(relayUrl);
  // Encode the normalized relay so it can't contain the `:` delimiter.
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalized)}`;
}

/** Parses the legacy `sections`/`assignments` fields; orphaned assignments are dropped. */
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
  const sectionIds = new Set(sections.map((s) => s.id));
  const assignments: Record<string, string> =
    typeof obj.assignments === "object" &&
    obj.assignments !== null &&
    !Array.isArray(obj.assignments)
      ? Object.fromEntries(
          Object.entries(obj.assignments as Record<string, unknown>).filter(
            (entry): entry is [string, string] =>
              typeof entry[1] === "string" && sectionIds.has(entry[1]),
          ),
        )
      : {};
  return { version: 1, sections, assignments };
}

import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";
import {
  clearOwnOutbox,
  markLegacyConsumed,
  reclaimOutbox,
  resumeWholeBlobOutbox,
  writeOwnOutbox,
} from "./sidebarSyncWatermark";
import { claimLegacy } from "./mergeLaneStorage.shared";

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
 * sections from different communities/relays don't bleed across each other.
 * When omitted the legacy pubkey-only key is returned (used only during
 * one-time migration in `readChannelSectionsStore`).
 */
export function storageKey(pubkey: string, relayUrl?: string): string {
  if (!relayUrl) return `${STORAGE_KEY_PREFIX}:${pubkey}`;
  const normalized = normalizeRelayUrl(relayUrl);
  // Encode the normalized relay so it can't contain the `:` delimiter.
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalized)}`;
}

export function boundChannelSectionsStore(
  store: ChannelSectionStore,
): ChannelSectionStore {
  const sections = store.sections
    .slice()
    .sort((left, right) => left.order - right.order)
    .slice(-MAX_CHANNEL_SECTIONS);
  const sectionIds = new Set(sections.map((section) => section.id));
  const assignments = Object.fromEntries(
    Object.entries(store.assignments)
      .filter(([, sectionId]) => sectionIds.has(sectionId))
      .slice(-MAX_CHANNEL_SECTION_ASSIGNMENTS),
  );
  if (
    sections.length === store.sections.length &&
    Object.keys(assignments).length === Object.keys(store.assignments).length
  ) {
    return store;
  }
  return { ...store, sections, assignments };
}

export function stripOrphanedAssignments(
  store: ChannelSectionStore,
): ChannelSectionStore {
  const sectionIds = new Set(store.sections.map((s) => s.id));
  const cleaned = Object.fromEntries(
    Object.entries(store.assignments).filter(([, sid]) => sectionIds.has(sid)),
  );
  const stripped =
    Object.keys(cleaned).length === Object.keys(store.assignments).length
      ? store
      : { ...store, assignments: cleaned };
  return boundChannelSectionsStore(stripped);
}

export function parseChannelSectionPayload(
  json: unknown,
): ChannelSectionStore | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  // Reject payloads from a future schema version we cannot safely interpret:
  // a v2+ blob on the relay must trigger the retain/retry path so a later local
  // edit never publishes over authoritative state this client does not understand
  // (matches sort/stars/mutes parsers and Carl P1). Local-storage reads always
  // go through `parseRaw`, which checks `version !== 1` before calling here, so
  // this guard does not change the local-storage parse contract.
  if (obj.version !== 1) return null;
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
 * Enforces one invariant on every read: a scoped copy is exposed only while no
 * importable legacy (pubkey-only) key remains. If a non-empty legacy key still
 * exists — whether because this is the first scoped read or because a prior
 * migration wrote the scoped copy but could not delete the legacy key — we
 * finish the claim inline: write the scoped copy if absent, delete the legacy
 * key, and expose the value only once that delete provably took. On any storage
 * failure we return `DEFAULT_STORE`, which cannot seed-publish (bootstrap gates
 * on a non-empty local store), leaving the claim to complete on a later read
 * once storage recovers. This makes "scoped data is owned only when the legacy
 * key is gone" a property the reader proves every read, not a promise a
 * one-shot migration hopes to keep under failing storage — so relay A's
 * sections can never seed-publish onto a first-visited relay B (Carl P1).
 *
 * Concurrent windows are safe without an atomic claim. Each window (main and
 * huddle) takes its scope from the app-wide active community at mount, so the
 * first scoped read for a live legacy key runs before any second relay scope
 * can exist; that read either claims the legacy key or hits the failure path
 * above. A race migrates the identical legacy value to the identical scoped key
 * (idempotent), and the loser's post-delete confirmation fails and exposes
 * `DEFAULT_STORE`.
 */
export function readChannelSectionsStore(
  pubkey: string,
  relayUrl?: string,
): ChannelSectionStore {
  try {
    const key = storageKey(pubkey, relayUrl);
    const scoped = parseRaw(window.localStorage.getItem(key));

    // Unscoped read: the key IS the legacy key, so there is nothing to claim.
    if (!relayUrl) return scoped ?? DEFAULT_STORE;

    const legacyKey = storageKey(pubkey);
    const legacy = parseRaw(window.localStorage.getItem(legacyKey));
    const legacyHasData = legacy !== null && legacy.sections.length > 0;

    // No importable legacy value remains — an existing scoped copy is proven
    // owned. This short-circuits every read after a completed claim.
    if (!legacyHasData) return scoped ?? DEFAULT_STORE;

    // A consumable legacy key still coexists with (or would seed) the scoped
    // key; neither is safe to expose until that legacy key is provably gone.
    return claimLegacy(
      key,
      legacyKey,
      scoped ?? legacy,
      scoped !== null,
      DEFAULT_STORE,
    );
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
      JSON.stringify(boundChannelSectionsStore(store)),
    );
    return true;
  } catch {
    return false;
  }
}

const OUTBOX_KEY_PREFIX = "buzz-channel-sections-outbox.v1";

// The single shared key written by builds before the outbox was keyed
// per-window. Enumerated as one more record so an edit persisted by a prior
// build still resumes, and reclaimed by the same relay-gated rule.
function legacyOutboxKey(pubkey: string, relayUrl: string): string {
  return `${OUTBOX_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

/**
 * Persist this window's unpublished edit under its own outbox key. Written
 * synchronously on every edit as a single unconditional `setItem` (no shared-
 * key read-modify-write); resumed on next mount so an edit made <2s before
 * quit/community-switch is never dropped. `queuedAt` stamps the write so resume
 * replays only the newest queued blob (whole-blob LWW). Returns whether the
 * intent is now durably held in this window's own v2 key (see `writeOwnOutbox`).
 */
export function writeChannelSectionsOutbox(
  pubkey: string,
  store: ChannelSectionStore,
  relayUrl: string,
  nowSecs?: number,
): boolean {
  return writeOwnOutbox(
    OUTBOX_KEY_PREFIX,
    pubkey,
    relayUrl,
    boundChannelSectionsStore(store),
    nowSecs,
  );
}

/**
 * The whole-blob outbox record to resume on boot, or null when none exists.
 * Whole-blob LWW: only the max-`queuedAt` record is replayed. Returns the
 * winning store plus, when that winner is a not-yet-consumed legacy blob, the
 * raw string the caller marks consumed (via `markChannelSectionsLegacyConsumed`)
 * once it has durably re-queued the intent — the legacy key is never deleted,
 * so this one-shot marker is what stops it republishing above the head forever.
 */
export function readChannelSectionsOutbox(
  pubkey: string,
  relayUrl: string,
): {
  store: ChannelSectionStore;
  legacyRawToConsume: string | null;
  queuedAt: number;
} | null {
  return resumeWholeBlobOutbox(
    OUTBOX_KEY_PREFIX,
    legacyOutboxKey(pubkey, relayUrl),
    pubkey,
    relayUrl,
    parseChannelSectionPayload,
  );
}

/**
 * Mark a replayed legacy sections blob consumed so it is not resumed again on a
 * later boot. Call only AFTER the intent is durably held in this window's own
 * v2 key (its synchronous publish path), so a crash before this write replays
 * the legacy blob once more rather than losing it.
 */
export function markChannelSectionsLegacyConsumed(
  pubkey: string,
  relayUrl: string,
  raw: string,
): void {
  markLegacyConsumed(OUTBOX_KEY_PREFIX, pubkey, relayUrl, raw);
}

/** Clear this window's own outbox key (its edit published or is a no-op). */
export function clearChannelSectionsOutbox(
  pubkey: string,
  relayUrl: string,
): void {
  clearOwnOutbox(OUTBOX_KEY_PREFIX, pubkey, relayUrl);
}

/**
 * Reclaim foreign outbox keys the relay head itself STRICTLY supersedes: a
 * whole-blob record queued strictly before the durable head's `created_at`
 * (`queuedAt` < `headCreatedAt`) lost LWW to a blob the relay already holds, so
 * dropping it matches the relay's own resolution. A same-second record
 * (`queuedAt` == `headCreatedAt`) is kept — one-second clock granularity cannot
 * prove it lost, so it drains only when a strictly-newer head lands. A record
 * queued after the head is live intent and is likewise kept. Records are
 * write-once so the delete needs no recheck; never touches this window's own
 * keys or the legacy shared key. Call only after a successful fetch.
 */
export function reclaimSupersededSectionsOutbox(
  pubkey: string,
  relayUrl: string,
  headCreatedAt: number,
): void {
  reclaimOutbox(
    OUTBOX_KEY_PREFIX,
    legacyOutboxKey(pubkey, relayUrl),
    pubkey,
    relayUrl,
    parseChannelSectionPayload,
    (record) => record.queuedAt < headCreatedAt,
  );
}

import type { InboxFilter } from "@/features/home/lib/inbox";
import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

const STORAGE_KEY_PREFIX = "buzz-inbox-view.v1";

const INBOX_FILTERS: ReadonlySet<string> = new Set([
  "all",
  "project",
  "mention",
  "thread",
  "needs_action",
  "agent_activity",
  "reminders",
  "drafts",
]);

/** Durable Inbox view settings scoped to one user and relay. */
export type InboxViewPreference = {
  version: 1;
  filter: InboxFilter;
  unreadOnly: boolean;
};

export const DEFAULT_INBOX_VIEW_PREFERENCE: InboxViewPreference = Object.freeze(
  {
    version: 1,
    filter: "all",
    unreadOnly: false,
  },
);

/** Returns the local cache key for one user's Inbox view settings. */
export function inboxViewPreferenceStorageKey(
  pubkey: string,
  relayUrl?: string,
): string {
  if (!relayUrl) return `${STORAGE_KEY_PREFIX}:${pubkey}`;
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

/** Parses the versioned local or NIP-78 Inbox preference payload. */
export function parseInboxViewPreference(
  json: unknown,
): InboxViewPreference | null {
  if (typeof json !== "object" || json === null) return null;
  const obj = json as Record<string, unknown>;
  if (obj.version !== 1) return null;

  return {
    version: 1,
    filter:
      typeof obj.filter === "string" && INBOX_FILTERS.has(obj.filter)
        ? (obj.filter as InboxFilter)
        : DEFAULT_INBOX_VIEW_PREFERENCE.filter,
    unreadOnly:
      typeof obj.unreadOnly === "boolean"
        ? obj.unreadOnly
        : DEFAULT_INBOX_VIEW_PREFERENCE.unreadOnly,
  };
}

/** Reads the instant/offline Inbox preference cache. */
export function readInboxViewPreference(
  pubkey: string,
  relayUrl?: string,
): InboxViewPreference {
  try {
    const raw = window.localStorage.getItem(
      inboxViewPreferenceStorageKey(pubkey, relayUrl),
    );
    if (!raw) return DEFAULT_INBOX_VIEW_PREFERENCE;
    return (
      parseInboxViewPreference(JSON.parse(raw)) ?? DEFAULT_INBOX_VIEW_PREFERENCE
    );
  } catch {
    return DEFAULT_INBOX_VIEW_PREFERENCE;
  }
}

/** Writes the instant/offline Inbox preference cache. */
export function writeInboxViewPreference(
  pubkey: string,
  preference: InboxViewPreference,
  relayUrl?: string,
): boolean {
  try {
    window.localStorage.setItem(
      inboxViewPreferenceStorageKey(pubkey, relayUrl),
      JSON.stringify(preference),
    );
    return true;
  } catch {
    return false;
  }
}

/**
 * inboxReplyMigration.ts
 *
 * Testable production helper for migrating legacy `inbox-reply:<eventId>`
 * draft keys to the canonical `thread:<conversationId>` shape introduced by
 * the stable inbox conversation identity work.
 *
 * The migration is triggered at DraftsPanel open/send-confirm time (the only
 * point where both the clicked draft key and navigation intent are known). It
 * is extracted here so unit tests can exercise the full decision tree without
 * a React component.
 */

import type { RelayEvent } from "@/shared/api/types";
import type { DraftState } from "@/features/messages/lib/useDrafts";

export const INBOX_REPLY_PREFIX = "inbox-reply:";
export const THREAD_DRAFT_PREFIX = "thread:";

/**
 * The successful result of a legacy draft migration. Both fields are needed
 * by the component to navigate to the correct thread composer.
 */
export type InboxReplyMigrationResult = {
  /** The canonical `thread:` draft key to use for the composer. */
  newDraftKey: string;
  /** The derived conversationId (rootId ?? parentId ?? eventId). */
  conversationId: string;
  /** The channel ID from the resolved event (confirmed to match storedChannelId). */
  channelId: string;
};

/**
 * Attempt to migrate a legacy `inbox-reply:<eventId>` draft key.
 *
 * Returns `InboxReplyMigrationResult` on success (rekey performed, navigate).
 * Returns `null` on any failure — malformed key, trim-to-empty ID, resolution
 * failure, event-ID mismatch, channel-tag mismatch, or key collision — leaving
 * the legacy draft untouched.
 *
 * @param legacyKey    The full draft key, must start with `inbox-reply:`.
 * @param storedDraft  The DraftState stored under legacyKey.
 * @param deps         Injected dependencies (production or test doubles).
 */
export async function migrateInboxReplyDraft(
  legacyKey: string,
  storedDraft: DraftState,
  deps: {
    getEventById: (id: string) => Promise<RelayEvent>;
    getChannelIdFromTags: (tags: string[][]) => string | null;
    getThreadReference: (tags: string[][]) => {
      rootId: string | null;
      parentId: string | null;
    };
    renameDraftEntry: (
      oldKey: string,
      newKey: string,
    ) => "migrated" | "collision" | "noop";
  },
): Promise<InboxReplyMigrationResult | null> {
  if (!legacyKey.startsWith(INBOX_REPLY_PREFIX)) return null;

  const rawId = legacyKey.slice(INBOX_REPLY_PREFIX.length).trim();
  // Must be a 64-character hex string (Nostr event ID).
  if (!/^[0-9a-f]{64}$/i.test(rawId)) return null;

  let event: RelayEvent;
  try {
    event = await deps.getEventById(rawId);
  } catch {
    return null;
  }

  // Reject if the relay returned a different event than requested.
  if (event.id !== rawId) return null;

  const eventChannelId = deps.getChannelIdFromTags(event.tags);
  if (!eventChannelId || eventChannelId !== storedDraft.channelId) return null;

  const ref = deps.getThreadReference(event.tags);
  const conversationId = ref.rootId ?? ref.parentId ?? event.id;
  const newDraftKey = `${THREAD_DRAFT_PREFIX}${conversationId}`;

  const result = deps.renameDraftEntry(legacyKey, newDraftKey);
  if (result !== "migrated") return null;

  return { newDraftKey, conversationId, channelId: eventChannelId };
}

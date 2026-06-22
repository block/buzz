import { getThreadReference } from "@/features/messages/lib/threading";
import type { ReminderTarget } from "@/features/reminders/lib/reminderTypes";
import { getEventById } from "@/shared/api/tauri";

/**
 * Where a reminder click should land. Mirrors the `kind: "channel"` arm of the
 * search-hit destination: a message inside a channel, optionally inside a thread.
 */
export type ReminderDestination = {
  channelId: string;
  messageId: string;
  threadRootId: string | null;
};

/**
 * A target is navigable only when it carries a non-empty channelId, eventId,
 * and authorPubkey. The creation site stores `channelId ?? ""` /
 * `authorPubkey ?? ""`, so a *present* target can still hold empty strings —
 * those route to `/channels/` with an empty param or render a meaningless
 * author, and must be treated as non-navigable, same as note-only reminders
 * (no target at all).
 */
export function hasNavigableTarget(
  target: ReminderTarget | undefined,
): target is ReminderTarget {
  return (
    target !== undefined &&
    target.channelId !== "" &&
    target.eventId !== "" &&
    target.authorPubkey !== ""
  );
}

/**
 * Resolves the in-thread destination for a reminder target.
 *
 * Reminder targets store no thread context, so — like the forum-comment branch
 * of `resolveSearchHitDestination` — we fetch the target event and derive its
 * thread root from the tags. A top-level (non-reply) message yields a null root
 * (channel-level, no thread to enter); a fetch failure degrades the same way.
 *
 * Returns null when the target is non-navigable (absent or empty fields).
 */
export async function resolveReminderDestination(
  target: ReminderTarget | undefined,
  fetchEvent: typeof getEventById = getEventById,
): Promise<ReminderDestination | null> {
  if (!hasNavigableTarget(target)) {
    return null;
  }

  try {
    const event = await fetchEvent(target.eventId);
    return {
      channelId: target.channelId,
      messageId: target.eventId,
      threadRootId: getThreadReference(event.tags).rootId,
    };
  } catch (error) {
    console.error(
      "Failed to resolve reminder thread destination",
      target.eventId,
      error,
    );
    return {
      channelId: target.channelId,
      messageId: target.eventId,
      threadRootId: null,
    };
  }
}

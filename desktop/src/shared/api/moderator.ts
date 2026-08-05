/**
 * Moderator-specific API commands.
 *
 * These commands are available to relay owners, admins, and moderators. They
 * use Buzz-native event kinds so the relay can validate authority via the
 * moderation seam (authorize_moderation_action).
 */
import { invokeTauri } from "@/shared/api/tauri";

/**
 * Delete a message as a relay-level moderator, owner, or admin (kind 9005).
 *
 * Unlike the standard delete (kind 5, self-authored only), this sends a
 * Buzz-native moderator delete validated against the actor's relay role. Use
 * when deleting a message you did not author.
 */
export async function moderatorDeleteMessage(
  channelId: string,
  eventId: string,
): Promise<void> {
  await invokeTauri("moderator_delete_message", { channelId, eventId });
}

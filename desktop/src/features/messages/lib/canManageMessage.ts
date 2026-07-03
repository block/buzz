import { KIND_HUDDLE_STARTED } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";

/**
 * Returns true when the current user may edit or delete `message`.
 *
 * Two paths grant permission — mirroring the relay's authz:
 *   1. Self-author: the current user pubkey matches the message pubkey.
 *   2. Owner-of-agent: the message author's profile carries an `ownerPubkey`
 *      (NIP-OA owner record) equal to the current user's pubkey.
 *
 * Huddle-started messages are immutable regardless of authorship.
 */
export function canManageMessageForCurrentUser(
  message: TimelineMessage,
  currentPubkey: string | undefined,
  profiles: UserProfileLookup | undefined,
): boolean {
  if (message.kind === KIND_HUDDLE_STARTED) return false;
  if (!currentPubkey || !message.pubkey) return false;
  if (normalizePubkey(message.pubkey) === normalizePubkey(currentPubkey))
    return true;
  return ownsAuthorAgent(
    profiles?.[normalizePubkey(message.pubkey)],
    currentPubkey,
  );
}

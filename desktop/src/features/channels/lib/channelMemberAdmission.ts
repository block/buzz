/**
 * Client mirror of the relay's kind:9000 authority for adding *another*
 * identity to a channel (`buzz_core::channel::decide_member_put`):
 *
 * - DMs: nobody — membership is fixed at creation.
 * - Guests: never. A guest can read and leave but not write, and adding
 *   people is a write.
 * - Open channels: anyone else, member or not.
 * - Private channels: any other active member. The relay separately reserves
 *   elevated role grants and active-role changes for owners/admins.
 *
 * Unknown visibility fails closed — the relay is the authority and a hidden
 * button is cheaper than an opaque rejection.
 */
export function canAddChannelMembers({
  channelType,
  visibility,
  selfRole,
}: {
  channelType?: string | null;
  visibility?: string | null;
  selfRole?: string | null;
}): boolean {
  if (channelType === "dm" || selfRole === "guest") {
    return false;
  }

  if (visibility === "open") {
    return true;
  }

  if (visibility === "private") {
    return selfRole != null;
  }

  return false;
}

/** Explains a denied add so the user isn't left guessing at a missing button. */
export const PRIVATE_CHANNEL_ADD_DENIED_MESSAGE =
  "Only channel members who can post can add people to a private channel.";

/**
 * Client mirror of the relay's kind:9000 authority for adding *another*
 * identity to a channel (`validate_admin_event` + `buzz_db::channel::add_member`):
 *
 * - DMs: nobody — membership is fixed at creation.
 * - Open channels: anyone, member or not.
 * - Private channels: any active member. The relay separately reserves elevated
 *   role grants and active-role changes for owners/admins.
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
  if (channelType === "dm") {
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
  "Only channel members can add people to a private channel.";

/**
 * Client mirror of the relay's kind:9000 authority for adding *another*
 * identity to a channel (`validate_admin_event` + `buzz_db::channel::add_member`):
 *
 * - DMs: nobody — membership is fixed at creation.
 * - Open channels: anyone, member or not.
 * - Private channels: owners/admins only. A plain member extending access to
 *   channel history is exactly what the relay now rejects, so the affordance
 *   must not be offered.
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

  return selfRole === "owner" || selfRole === "admin";
}

/** Explains a denied add so the user isn't left guessing at a missing button. */
export const PRIVATE_CHANNEL_ADD_DENIED_MESSAGE =
  "Only channel owners and admins can add people to a private channel.";

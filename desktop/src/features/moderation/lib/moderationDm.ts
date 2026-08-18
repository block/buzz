import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * A moderation DM is the 1:1 direct message between a member and the relay's
 * own identity — the channel moderators use to explain an action. The member
 * must not be able to reply into it, so the composer is disabled on this
 * channel alone (never on ordinary DMs).
 *
 * Identification is client-side and best-effort: the relay identity is the
 * NIP-11 `self` pubkey (see {@link getRelaySelf}), and a moderation DM is a DM
 * whose only other participant is that pubkey. It is an affordance, not
 * enforcement — the relay decides what a write does — so this fails open: a
 * missing `relaySelf` (relay unreachable, no `self` advertised) yields `false`,
 * leaving the composer enabled.
 */
export function isModerationDm(
  channel: Pick<Channel, "channelType" | "participantPubkeys"> | null,
  currentPubkey: string | undefined,
  relaySelf: string | null | undefined,
): boolean {
  if (channel?.channelType !== "dm" || !relaySelf) {
    return false;
  }
  const self = normalizePubkey(relaySelf);
  const me = currentPubkey ? normalizePubkey(currentPubkey) : null;
  const others = channel.participantPubkeys
    .map(normalizePubkey)
    .filter((pubkey) => pubkey !== me);
  return others.length === 1 && others[0] === self;
}

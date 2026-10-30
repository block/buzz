import type { Channel } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * The cached DM channel whose only other participant is `targetPubkey`.
 *
 * Reusing the existing conversation matters for any action that writes into a
 * DM: opening a fresh one would strand the message in a second thread the
 * recipient reads separately. Group DMs are deliberately excluded — a 1:1
 * action must not leak into a wider audience.
 */
export function findCachedOneToOneDm(
  channels: readonly Channel[] | undefined,
  targetPubkey: string,
  currentPubkey: string | undefined,
): Channel | null {
  const normalizedTargetPubkey = normalizePubkey(targetPubkey);
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;

  return (
    channels?.find((channel) => {
      if (channel.channelType !== "dm") {
        return false;
      }

      const participantPubkeys =
        channel.participantPubkeys.map(normalizePubkey);
      if (!participantPubkeys.includes(normalizedTargetPubkey)) {
        return false;
      }

      const otherParticipantPubkeys = normalizedCurrentPubkey
        ? participantPubkeys.filter(
            (participantPubkey) =>
              participantPubkey !== normalizedCurrentPubkey,
          )
        : participantPubkeys;

      return (
        otherParticipantPubkeys.length === 1 &&
        otherParticipantPubkeys[0] === normalizedTargetPubkey
      );
    }) ?? null
  );
}

/**
 * The 1:1 DM to write into for `targetPubkey`, opening one only when none is
 * cached.
 *
 * Callers pass `useOpenDmMutation().mutateAsync` as `openDm`; it is the
 * fallback rather than the first move so an action never splits an existing
 * conversation in two.
 */
export async function resolveOneToOneDm({
  channels,
  currentPubkey,
  openDm,
  targetPubkey,
}: {
  channels: readonly Channel[] | undefined;
  currentPubkey: string | undefined;
  openDm: (input: { pubkeys: string[] }) => Promise<Channel>;
  targetPubkey: string;
}): Promise<Channel> {
  return (
    findCachedOneToOneDm(channels, targetPubkey, currentPubkey) ??
    (await openDm({ pubkeys: [normalizePubkey(targetPubkey)] }))
  );
}

/**
 * Pubkeys already reachable through a cached 1:1 DM, for surfaces that offer
 * both channels and people: the DM is the channel row, so the same person must
 * not also appear as a "start a new DM" row.
 */
export function oneToOneDmPubkeys(
  channels: readonly Channel[] | undefined,
  currentPubkey: string | undefined,
): Set<string> {
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  const pubkeys = new Set<string>();

  for (const channel of channels ?? []) {
    if (channel.channelType !== "dm") continue;
    const others = [
      ...new Set(channel.participantPubkeys.map(normalizePubkey)),
    ].filter((pubkey) => pubkey && pubkey !== normalizedCurrentPubkey);
    if (others.length === 1) pubkeys.add(others[0]);
  }

  return pubkeys;
}

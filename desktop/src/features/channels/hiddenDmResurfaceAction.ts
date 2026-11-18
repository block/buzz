import type { ChannelMember, RelayEvent } from "@/shared/api/types";
import type { OpenDmInput } from "@/shared/api/tauriChannels";
import {
  dmPeerPubkeysFromMembers,
  isIncomingChannelMessageFromOther,
  relayEventChannelId,
} from "./dmResurface";

type HiddenDmResurfaceActionOptions = {
  event: RelayEvent;
  expectedRelayUrl: string;
  expectedSignerPubkey: string;
  hiddenDmIds: ReadonlySet<string>;
  fetchMembers: (channelId: string) => Promise<readonly ChannelMember[]>;
  isCurrent: () => boolean;
  reopen: (input: OpenDmInput) => Promise<{ id: string }>;
};

export async function resurfaceHiddenDmMessage({
  event,
  expectedRelayUrl,
  expectedSignerPubkey,
  hiddenDmIds,
  fetchMembers,
  isCurrent,
  reopen,
}: HiddenDmResurfaceActionOptions): Promise<boolean> {
  if (!isIncomingChannelMessageFromOther(event, expectedSignerPubkey)) {
    return false;
  }
  const channelId = relayEventChannelId(event);
  if (!channelId || !hiddenDmIds.has(channelId)) return false;

  const members = await fetchMembers(channelId);
  if (!isCurrent()) return false;
  const pubkeys = dmPeerPubkeysFromMembers(members, expectedSignerPubkey);
  if (pubkeys.length === 0) return false;

  const opened = await reopen({
    pubkeys,
    expectedRelayUrl,
    expectedSignerPubkey,
  });
  if (!isCurrent()) return false;
  if (opened.id !== channelId) {
    throw new Error("Relay reopened a different DM conversation.");
  }
  return true;
}

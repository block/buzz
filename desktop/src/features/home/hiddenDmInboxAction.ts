import { dmPeerPubkeysFromMembers } from "@/features/channels/dmResurface";
import type { InboxItem } from "@/features/home/lib/inbox";
import type { ChannelMember } from "@/shared/api/types";
import type { OpenDmInput } from "@/shared/api/tauriChannels";

type HiddenDmInboxActionOptions = {
  item: InboxItem;
  channelId: string;
  messageId: string;
  threadRootId?: string | null;
  availableChannelIds: ReadonlySet<string>;
  expectedRelayUrl: string;
  expectedSignerPubkey: string;
  pendingChannelIds: Set<string>;
  fetchMembers: (channelId: string) => Promise<readonly ChannelMember[]>;
  openDm: (input: OpenDmInput) => Promise<{ id: string }>;
  isCurrent: () => boolean;
  onOpenContext: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
  onError: () => void;
  onPendingChange: () => void;
};

export async function openHiddenDmInboxContext({
  item,
  channelId,
  messageId,
  threadRootId,
  availableChannelIds,
  expectedRelayUrl,
  expectedSignerPubkey,
  pendingChannelIds,
  fetchMembers,
  openDm,
  isCurrent,
  onOpenContext,
  onError,
  onPendingChange,
}: HiddenDmInboxActionOptions): Promise<boolean> {
  if (availableChannelIds.has(channelId) || item.item.channelType !== "dm") {
    if (isCurrent()) onOpenContext(channelId, messageId, threadRootId);
    return true;
  }
  if (pendingChannelIds.has(channelId)) return false;

  pendingChannelIds.add(channelId);
  onPendingChange();
  try {
    const members = await fetchMembers(channelId);
    if (!isCurrent()) return false;
    const pubkeys = dmPeerPubkeysFromMembers(members, expectedSignerPubkey);
    if (pubkeys.length === 0) {
      throw new Error("Could not determine the DM membership.");
    }
    const opened = await openDm({
      pubkeys,
      expectedRelayUrl,
      expectedSignerPubkey,
    });
    if (!isCurrent()) return false;
    if (opened.id !== channelId) {
      throw new Error("Relay reopened a different DM conversation.");
    }
    onOpenContext(channelId, messageId, threadRootId);
    return true;
  } catch {
    if (isCurrent()) onError();
    return false;
  } finally {
    pendingChannelIds.delete(channelId);
    if (isCurrent()) onPendingChange();
  }
}

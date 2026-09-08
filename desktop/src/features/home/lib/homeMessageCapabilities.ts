import type { InboxItem } from "@/features/home/lib/inbox";

export function getHomeMessageCapabilities(
  item: InboxItem | null,
  currentPubkey: string | undefined,
  availableChannelIds: ReadonlySet<string>,
) {
  const canReact = Boolean(
    item?.item.channelId && availableChannelIds.has(item.item.channelId),
  );
  const canReply =
    canReact && item?.item.kind !== 45001 && item?.item.kind !== 45003;
  const disabledReplyReason =
    canReply || !item
      ? null
      : item.item.channelId
        ? availableChannelIds.has(item.item.channelId)
          ? "This item does not support inline replies yet."
          : "Open the linked channel to reply."
        : "This inbox item does not have a reply target.";

  return {
    canDelete:
      item !== null &&
      currentPubkey?.trim().toLowerCase() ===
        item.item.pubkey.trim().toLowerCase(),
    canReact,
    canReply,
    disabledReplyReason,
  };
}

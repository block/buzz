import { i18n } from "@/i18n";
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
          ? i18n.t("home.inbox.reply-no-inline")
          : i18n.t("home.inbox.reply-open-linked")
        : i18n.t("home.inbox.reply-no-target");

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

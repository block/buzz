import { canReadMessageEmbedSource } from "@/features/messages/lib/messageEmbed";
import { cn } from "@/shared/lib/cn";
import {
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
} from "@/shared/ui/mentionChip";

import type { MessageLinkPillProps } from "./types";

export function MessageLinkPill({
  channels,
  href,
  interactive,
  link,
  onOpenMessageLink,
}: MessageLinkPillProps) {
  const channel = channels.find((c) => c.id === link.channelId);
  const canRead = canReadMessageEmbedSource(channel);
  const channelLabel = canRead
    ? (channel?.name ?? "channel")
    : "private channel";
  const shortId = link.messageId.slice(0, 6);
  const label = canRead ? (
    <>
      #{channelLabel} · {shortId}
    </>
  ) : (
    <>Private message or channel</>
  );

  if (!interactive || !canRead) {
    return (
      <span
        className={cn(!canRead && MENTION_CHIP_BASE_CLASSES)}
        data-message-link=""
      >
        {label}
      </span>
    );
  }

  return (
    <button
      type="button"
      data-message-link=""
      aria-label={`Open message in ${channelLabel}`}
      title={href}
      className={cn(
        "cursor-pointer",
        MENTION_CHIP_BASE_CLASSES,
        MENTION_CHIP_HOVER_CLASSES,
      )}
      onClick={() => {
        onOpenMessageLink(link);
      }}
    >
      {label}
    </button>
  );
}

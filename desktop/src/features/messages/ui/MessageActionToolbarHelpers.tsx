import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { TimelineMessage } from "@/features/messages/types";
import { KIND_HUDDLE_STARTED } from "@/shared/constants/kinds";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { emojiDisplayName } from "@/shared/lib/emojiName";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function copyMessageLink(channelId: string, message: TimelineMessage) {
  const { rootId } = getThreadReference(message.tags ?? []);
  const link = buildMessageLink({
    channelId,
    messageId: message.id,
    threadRootId: rootId,
  });
  copyTextToClipboard(link, "Link copied to clipboard");
}

export function canCopyMessageLink(
  message: TimelineMessage,
  channelId: string | null | undefined,
): channelId is string {
  return (
    !message.pending &&
    message.kind !== KIND_HUDDLE_STARTED &&
    Boolean(channelId)
  );
}

export function QuickReactionButton({
  customEmojiUrl,
  emoji,
  onSelect,
}: {
  customEmojiUrl?: string;
  emoji: string;
  onSelect: (emoji: string) => void;
}) {
  const displayName = emojiDisplayName(emoji);
  const mediaUrl = customEmojiUrl ? rewriteRelayUrl(customEmojiUrl) : null;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={`React with ${displayName}`}
          className="flex h-8 w-8 items-center justify-center rounded-full text-base leading-none text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          onClick={() => onSelect(emoji)}
          title={displayName}
          type="button"
        >
          {mediaUrl ? (
            <img
              alt={emoji}
              className="h-5 w-5 object-contain"
              draggable={false}
              src={mediaUrl}
            />
          ) : (
            <span aria-hidden="true" className="translate-y-px">
              {emoji}
            </span>
          )}
        </button>
      </TooltipTrigger>
      <TooltipContent>{displayName}</TooltipContent>
    </Tooltip>
  );
}

export function isCustomEmojiShortcode(emoji: string) {
  return emoji.startsWith(":") && emoji.endsWith(":");
}

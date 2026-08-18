import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { getSentFromThreadReference } from "@/features/messages/lib/sentFromThread";
import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
import { useChannelNavigation } from "@/shared/context/ChannelNavigationContext";
import { MessageLinkPill } from "@/shared/ui/markdown/MessageLinkPill";
import { MESSAGE_MARKDOWN_CLASS } from "@/shared/ui/mentionChip";

export function SentFromThreadLine({
  channelId,
  tags,
}: {
  channelId?: string | null;
  tags?: string[][];
}) {
  const { channels } = useChannelNavigation();
  const { goChannel } = useAppNavigation();
  const reference = getSentFromThreadReference(tags);
  const onOpenMessageLink = React.useCallback(
    (target: ParsedMessageLink) => {
      void goChannel(target.channelId, {
        messageId: target.messageId,
        threadRootId: target.threadRootId,
      });
    },
    [goChannel],
  );

  if (!channelId || !reference) return null;
  const link: ParsedMessageLink = {
    channelId,
    messageId: reference.rootEventId,
    threadRootId: reference.rootEventId,
  };

  return (
    <div
      className={`${MESSAGE_MARKDOWN_CLASS} mb-1 flex min-h-[var(--inline-chip-min-height)] min-w-0 items-center gap-1.5 pt-0.5 text-sm font-normal leading-4 text-muted-foreground/70`}
      data-testid="sent-from-thread"
    >
      <span className="shrink-0">Sent from thread:</span>
      <MessageLinkPill
        channels={channels}
        interactive
        link={link}
        onOpenMessageLink={onOpenMessageLink}
        threadExcerpt={reference.rootExcerpt}
        variant="sent-from-thread"
      />
    </div>
  );
}

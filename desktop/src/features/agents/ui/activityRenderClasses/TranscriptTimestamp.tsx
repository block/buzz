import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { cn } from "@/shared/lib/cn";
import { formatTranscriptTime } from "../agentSessionUtils";

const fullDateTimeFormat = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  second: "2-digit",
});

export type TranscriptTimestampMessageLink = {
  channelId: string;
  messageId: string;
};

export function TranscriptTimestamp({
  className,
  messageLink = null,
  timestamp,
}: {
  className?: string;
  messageLink?: TranscriptTimestampMessageLink | null;
  timestamp: string;
}) {
  const formatted = formatTranscriptTime(timestamp);
  const { goChannel } = useAppNavigation();
  const href = messageLink ? buildMessageLink(messageLink) : null;
  const openMessage = React.useCallback(
    (event: React.MouseEvent<HTMLAnchorElement>) => {
      if (!messageLink) return;
      event.preventDefault();
      event.stopPropagation();
      void goChannel(messageLink.channelId, {
        messageId: messageLink.messageId,
      });
    },
    [goChannel, messageLink],
  );

  if (!formatted) return null;
  const date = new Date(timestamp);
  const fullDateTime = Number.isNaN(date.getTime())
    ? timestamp
    : fullDateTimeFormat.format(date);

  if (href) {
    return (
      <a
        className={cn(
          "shrink-0 rounded-sm text-xs text-muted-foreground/60 transition-colors hover:text-muted-foreground/80 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
          className,
        )}
        data-testid="transcript-open-message-link"
        href={href}
        onClick={openMessage}
        title={fullDateTime}
      >
        {formatted}
      </a>
    );
  }

  return (
    <span
      className={cn(
        "shrink-0 cursor-default text-xs text-muted-foreground/60",
        className,
      )}
      title={fullDateTime}
    >
      {formatted}
    </span>
  );
}

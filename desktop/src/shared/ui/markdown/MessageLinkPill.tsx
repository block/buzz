import * as React from "react";

import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { cn } from "@/shared/lib/cn";

import { BuzzLinkChip } from "./BuzzLinkChip";
import type { MessageLinkPillProps } from "./types";
import { getMessageLinkLabel } from "@/features/messages/lib/messageLinkLabel";

const graphemeSegmenter =
  typeof Intl.Segmenter === "function"
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : null;
const emojiGraphemePattern =
  /(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|[\uFE0F\u20E3])/u;

function segmentLinkLabel(label: string): Array<{
  isEmoji: boolean;
  start: number;
  text: string;
}> {
  const segments: Array<{ isEmoji: boolean; start: number; text: string }> = [];
  const graphemes = graphemeSegmenter
    ? Array.from(graphemeSegmenter.segment(label), ({ index, segment }) => ({
        start: index,
        text: segment,
      }))
    : Array.from(label, (text, start) => ({ start, text }));
  for (const { start, text } of graphemes) {
    const isEmoji = emojiGraphemePattern.test(text);
    const previous = segments.at(-1);
    if (previous?.isEmoji === isEmoji) {
      previous.text += text;
    } else {
      segments.push({ isEmoji, start, text });
    }
  }
  return segments;
}

export function MessageLinkPill({
  channels,
  href,
  interactive,
  link,
  onOpenMessageLink,
  threadExcerpt,
  variant = "default",
}: MessageLinkPillProps) {
  const [isHovered, setIsHovered] = React.useState(false);
  const channel = channels.find((c) => c.id === link.channelId);
  const channelLabel = channel?.name ?? link.channelId.slice(0, 8);
  const shortId = link.messageId.slice(0, 8);
  const isSentFromThread = variant === "sent-from-thread";
  const permalink = href ?? buildMessageLink(link);
  const label = getMessageLinkLabel({
    channelName: channelLabel,
    threadExcerpt,
    variant,
  });

  if (!isSentFromThread) {
    return (
      <BuzzLinkChip
        data-message-link=""
        href={permalink}
        icon="message"
        aria-label={`Open message ${shortId} in channel ${channelLabel}`}
        title={label}
        interactive={interactive}
        onOpenLink={() => {
          onOpenMessageLink(link);
        }}
      >
        {channelLabel} · {shortId}
      </BuzzLinkChip>
    );
  }

  if (!interactive) {
    return (
      <span className="inline-block max-w-80 truncate" data-message-link="">
        {label}
      </span>
    );
  }

  return (
    <button
      type="button"
      data-message-link=""
      data-hovered={isHovered ? "" : undefined}
      aria-label={`Open thread in ${channelLabel}`}
      title={label}
      className={cn(
        "max-w-80 cursor-pointer truncate",
        "inline-block min-w-0 text-left font-medium text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
      )}
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
      onClick={() => {
        onOpenMessageLink(link);
      }}
    >
      {segmentLinkLabel(label).map((segment) =>
        segment.isEmoji ? (
          <span key={segment.start} data-message-link-emoji="">
            {segment.text}
          </span>
        ) : (
          <span
            key={segment.start}
            className="transition-shadow"
            data-message-link-text=""
            style={{
              boxShadow: isHovered ? "inset 0 -1px 0 currentColor" : "none",
            }}
          >
            {segment.text}
          </span>
        ),
      )}
    </button>
  );
}

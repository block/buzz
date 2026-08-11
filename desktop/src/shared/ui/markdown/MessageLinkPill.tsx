import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
} from "@/shared/ui/mentionChip";

import type { MessageLinkPillProps } from "./types";
import {
  getMessageLinkChannelLabel,
  getMessageLinkLabel,
  MESSAGE_LINK_PREFIX,
} from "@/features/messages/lib/messageLinkLabel";

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
  interactive,
  link,
  onOpenMessageLink,
  threadExcerpt,
  variant = "default",
}: MessageLinkPillProps) {
  const [isHovered, setIsHovered] = React.useState(false);
  const channel = channels.find((c) => c.id === link.channelId);
  const channelLabel = channel?.name ?? "channel";
  const isSentFromThread = variant === "sent-from-thread";
  const label = getMessageLinkLabel({
    channelName: channelLabel,
    threadExcerpt,
    variant,
  });
  const channelLinkLabel = getMessageLinkChannelLabel(channelLabel);

  if (!interactive) {
    if (!isSentFromThread) {
      return (
        <span
          className="inline-flex min-w-0 max-w-80 items-center gap-1.5 align-baseline"
          data-message-link=""
        >
          <span className="shrink-0">{MESSAGE_LINK_PREFIX}</span>
          <span
            className={cn(
              MENTION_CHIP_BASE_CLASSES,
              "min-w-0 max-w-full truncate",
            )}
            data-channel-link=""
          >
            {channelLinkLabel}
          </span>
        </span>
      );
    }
    return (
      <span className="inline-block max-w-80 truncate" data-message-link="">
        {label}
      </span>
    );
  }

  if (!isSentFromThread) {
    return (
      <span
        className="inline-flex min-w-0 max-w-80 items-center gap-1.5 align-baseline"
        data-message-link=""
      >
        <span className="shrink-0">{MESSAGE_LINK_PREFIX}</span>
        <button
          type="button"
          aria-label={`Open thread in ${channelLabel}`}
          title={label}
          className={cn(
            MENTION_CHIP_BASE_CLASSES,
            MENTION_CHIP_HOVER_CLASSES,
            "min-w-0 max-w-full cursor-pointer truncate",
          )}
          data-channel-link=""
          onClick={() => {
            onOpenMessageLink(link);
          }}
        >
          {channelLinkLabel}
        </button>
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

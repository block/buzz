import * as React from "react";

import {
  isChannelReferenceOpenable,
  useChannelReference,
} from "@/features/channels/openChannelDirectory";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { getMessageLinkLabel } from "@/features/messages/lib/messageLinkLabel";
import { cn } from "@/shared/lib/cn";

import { BuzzLinkChip } from "./BuzzLinkChip";
import type { MessageLinkPillProps } from "./types";

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

function ResolvedMessageLinkPill(props: MessageLinkPillProps) {
  const channel = useChannelReference(props.link.channelId);
  const openable = isChannelReferenceOpenable(channel);
  return (
    <MessageLinkPillContents
      {...props}
      channelLabel={openable ? channel.name : undefined}
      openable={openable}
    />
  );
}

function MessageLinkPillContents({
  href,
  interactive,
  link,
  onOpenMessageLink,
  threadExcerpt,
  variant = "default",
  channelLabel: resolvedChannelLabel,
  openable = true,
}: MessageLinkPillProps & { channelLabel?: string; openable?: boolean }) {
  const [isHovered, setIsHovered] = React.useState(false);
  const channelLabel = resolvedChannelLabel ?? link.channelId.slice(0, 8);
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
        aria-label={
          openable
            ? `Open message ${shortId} in channel ${channelLabel}`
            : `Message ${shortId} in channel ${channelLabel}`
        }
        title={label}
        interactive={openable && interactive}
        onOpenLink={() => {
          if (openable) onOpenMessageLink(link);
        }}
      >
        {channelLabel} · {shortId}
      </BuzzLinkChip>
    );
  }

  if (!interactive || !openable) {
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
      onClick={() => onOpenMessageLink(link)}
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

export function MessageLinkPill(props: MessageLinkPillProps) {
  const knownChannel = props.channels?.find(
    (channel) => channel.id === props.link.channelId,
  );

  if (knownChannel || !props.resolveChannelReference) {
    return (
      <MessageLinkPillContents {...props} channelLabel={knownChannel?.name} />
    );
  }

  return <ResolvedMessageLinkPill {...props} />;
}

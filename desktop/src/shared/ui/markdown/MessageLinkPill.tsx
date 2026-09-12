import * as React from "react";

import {
  isChannelReferenceOpenable,
  useChannelReference,
} from "@/features/channels/openChannelDirectory";
import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { getMessageLinkLabel } from "@/features/messages/lib/messageLinkLabel";
import { cn } from "@/shared/lib/cn";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";
import { truncateInlineChipLabel } from "@/shared/ui/mentionChip";

import { BuzzLinkChip } from "./BuzzLinkChip";
import { useInlineTooltipPosition } from "./useInlineTooltipPosition";
import { useMessageLinkMetadata } from "./useMessageLinkMetadata";
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

function formatMessageAge(createdAt: number): string {
  const elapsedMinutes = Math.max(
    0,
    Math.floor((Date.now() - createdAt * 1_000) / 60_000),
  );
  if (elapsedMinutes < 1) return "just now";
  if (elapsedMinutes < 60) return `${elapsedMinutes}m ago`;
  const elapsedHours = Math.floor(elapsedMinutes / 60);
  if (elapsedHours < 24) return `${elapsedHours}h ago`;
  const elapsedDays = Math.floor(elapsedHours / 24);
  if (elapsedDays < 7) return `${elapsedDays}d ago`;
  return `${Math.floor(elapsedDays / 7)}w ago`;
}

function MessageLinkMetadataTooltip({
  children,
  footer,
  metadata,
}: {
  children: React.ReactElement;
  footer: string;
  metadata: ReturnType<typeof useMessageLinkMetadata>;
}) {
  const { contentRef, onPointerMove } = useInlineTooltipPosition();
  if (
    metadata.state.kind === "deleted" ||
    metadata.state.kind === "unavailable"
  ) {
    const message =
      metadata.state.kind === "deleted"
        ? "Message deleted"
        : "Message unavailable";
    return (
      <TooltipProvider delayDuration={500} skipDelayDuration={0}>
        <Tooltip>
          <TooltipTrigger asChild onPointerMove={onPointerMove}>
            {children}
          </TooltipTrigger>
          <TooltipContent ref={contentRef} side="top">
            {message}
          </TooltipContent>
        </Tooltip>
      </TooltipProvider>
    );
  }
  if (metadata.state.kind !== "ready" || !metadata.state.snippet.trim()) {
    return children;
  }
  const content = metadata.state.snippet;
  const sender = metadata.state.author;
  const age = formatMessageAge(metadata.state.createdAt);
  return (
    <TooltipProvider delayDuration={500} skipDelayDuration={0}>
      <Tooltip>
        <TooltipTrigger asChild onPointerMove={onPointerMove}>
          {children}
        </TooltipTrigger>
        <TooltipContent
          ref={contentRef}
          className="w-72 max-w-[min(18rem,calc(100vw-2rem))] px-3 py-2 text-left"
          side="top"
        >
          <span
            className="line-clamp-3 [overflow-wrap:anywhere] whitespace-normal"
            data-buzz-tooltip-metadata-content=""
          >
            {content}
          </span>
          <span
            className="mt-1 block max-w-full truncate whitespace-nowrap text-2xs text-secondary-foreground/80"
            data-buzz-tooltip-metadata-type=""
          >
            {footer}
            {sender ? ` · ${sender}` : null}
            {` · ${age}`}
          </span>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

function ResolvedMessageLinkPill(props: MessageLinkPillProps) {
  const channel = useChannelReference(props.link.channelId);
  const openable = isChannelReferenceOpenable(channel);
  return (
    <MessageLinkPillContents
      {...props}
      channel={openable ? channel : undefined}
      channelLabel={openable ? channel.name : undefined}
      openable={openable}
    />
  );
}

function MessageLinkPillContents({
  channel,
  href,
  interactive,
  link,
  onOpenChannel,
  onOpenMessageLink,
  threadExcerpt,
  variant = "default",
  channelLabel: resolvedChannelLabel,
  openable = true,
}: MessageLinkPillProps & {
  channel?: NonNullable<MessageLinkPillProps["channels"]>[number];
  channelLabel?: string;
  openable?: boolean;
}) {
  const [isHovered, setIsHovered] = React.useState(false);
  const channelLabel = resolvedChannelLabel ?? link.channelId.slice(0, 8);
  const isSentFromThread = variant === "sent-from-thread";
  const permalink = href ?? buildMessageLink(link);
  const shouldLoadMetadata = openable && interactive && variant === "default";
  const metadata = useMessageLinkMetadata(link, shouldLoadMetadata);
  const destination =
    channel?.channelType === "dm" ? channelLabel : `#${channelLabel}`;
  const tooltipFooter = link.threadRootId
    ? `Thread in ${destination}`
    : channel?.channelType === "dm"
      ? `Direct message with ${destination}`
      : channel?.channelType === "forum"
        ? `Forum post in ${destination}`
        : destination;
  const label = getMessageLinkLabel({
    channelName: channelLabel,
    threadExcerpt,
    variant,
  });

  if (!isSentFromThread) {
    // Keep fetched metadata and identity out of the visible label so resolution
    // never changes the chip width.
    const chipLabel = truncateInlineChipLabel(channelLabel);
    const isDeleted = metadata.state.kind === "deleted";
    const chip = (
      <BuzzLinkChip
        data-message-link=""
        data-message-link-state={isDeleted ? "deleted" : undefined}
        href={permalink}
        icon="message"
        aria-label={
          !openable
            ? `Message in channel ${channelLabel}`
            : isDeleted
              ? link.threadRootId
                ? `Open thread in channel ${channelLabel}; linked message was deleted`
                : `Open channel ${channelLabel}; linked message was deleted`
              : `Open message in channel ${channelLabel}`
        }
        className={cn(
          metadata.state.kind === "unavailable" && "buzz-link-unavailable",
          isDeleted && "buzz-link-deleted",
        )}
        interactive={openable && interactive}
        onOpenLink={() => {
          if (!openable) return;
          if (!isDeleted) {
            onOpenMessageLink(link);
            return;
          }
          if (link.threadRootId) {
            onOpenMessageLink({
              ...link,
              messageId: link.threadRootId,
              threadRootId: link.threadRootId,
            });
            return;
          }
          onOpenChannel(link.channelId);
        }}
        wrapping
      >
        {chipLabel}
      </BuzzLinkChip>
    );
    return interactive ? (
      <MessageLinkMetadataTooltip footer={tooltipFooter} metadata={metadata}>
        {chip}
      </MessageLinkMetadataTooltip>
    ) : (
      chip
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
      <MessageLinkPillContents
        {...props}
        channel={knownChannel}
        channelLabel={knownChannel?.name}
      />
    );
  }

  return <ResolvedMessageLinkPill {...props} />;
}

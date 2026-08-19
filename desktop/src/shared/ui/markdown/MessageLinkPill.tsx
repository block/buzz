import * as React from "react";

import { buildMessageLink } from "@/features/messages/lib/messageLink";
import { cn } from "@/shared/lib/cn";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

import { BuzzLinkChip } from "./BuzzLinkChip";
import { useInlineTooltipPosition } from "./useInlineTooltipPosition";
import { useMessageLinkMetadata } from "./useMessageLinkMetadata";
import type { MessageLinkPillProps } from "./types";
import { getMessageLinkLabel } from "@/features/messages/lib/messageLinkLabel";

const graphemeSegmenter =
  typeof Intl.Segmenter === "function"
    ? new Intl.Segmenter(undefined, { granularity: "grapheme" })
    : null;
const emojiGraphemePattern =
  /(?:\p{Extended_Pictographic}|\p{Regional_Indicator}|[\uFE0F\u20E3])/u;

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
  if (metadata.state.kind === "unavailable") {
    return (
      <TooltipProvider delayDuration={500} skipDelayDuration={0}>
        <Tooltip>
          <TooltipTrigger asChild onPointerMove={onPointerMove}>
            {children}
          </TooltipTrigger>
          <TooltipContent ref={contentRef} side="top">
            Message unavailable
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
          <span className="line-clamp-2" data-buzz-tooltip-metadata-content="">
            {content}
          </span>
          <span
            className="mt-1 block max-w-full truncate whitespace-nowrap text-2xs text-primary-foreground/70"
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
  const channelReadable =
    channel !== undefined &&
    (channel.isMember || channel.visibility === "open");
  const shouldLoadMetadata =
    channelReadable && interactive && variant === "default";
  const metadata = useMessageLinkMetadata(link, shouldLoadMetadata);
  const metadataPending =
    shouldLoadMetadata &&
    (metadata.state.kind === "idle" || metadata.state.kind === "loading");
  const inlineContext =
    metadata.state.kind === "ready"
      ? metadata.state.snippet
      : metadataPending
        ? link.messageId.slice(0, 8)
        : null;
  const isSentFromThread = variant === "sent-from-thread";
  const permalink = href ?? buildMessageLink(link);
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
    const chip = (
      <BuzzLinkChip
        data-message-link=""
        href={permalink}
        icon="message"
        aria-label={`Open message in channel ${channelLabel}`}
        className={cn(
          "max-w-64 overflow-hidden",
          metadata.state.kind === "unavailable" && "buzz-link-unavailable",
        )}
        interactive={interactive}
        onOpenLink={() => onOpenMessageLink(link)}
      >
        <span className="min-w-0 truncate">
          {channelLabel}
          {inlineContext ? ` · ${inlineContext}` : null}
        </span>
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

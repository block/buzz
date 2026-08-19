import type * as React from "react";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  buildChannelLink,
  parseChannelLink,
} from "@/features/messages/lib/channelLink";

import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

import { BuzzInlineLink, BuzzLinkChip } from "./BuzzLinkChip";
import { MessageLinkPill } from "./MessageLinkPill";
import { useMarkdownRuntime } from "./runtimeContext";
import { getReactNodeText } from "./utils";

function formatChannelActivity(timestamp: string): string | null {
  const activityAt = Date.parse(timestamp);
  if (!Number.isFinite(activityAt)) return null;
  const elapsedMinutes = Math.max(
    0,
    Math.floor((Date.now() - activityAt) / 60_000),
  );
  if (elapsedMinutes < 1) return "Active just now";
  if (elapsedMinutes < 60) return `Active ${elapsedMinutes}m ago`;
  const elapsedHours = Math.floor(elapsedMinutes / 60);
  if (elapsedHours < 24) return `Active ${elapsedHours}h ago`;
  const elapsedDays = Math.floor(elapsedHours / 24);
  if (elapsedDays < 7) return `Active ${elapsedDays}d ago`;
  return `Active ${Math.floor(elapsedDays / 7)}w ago`;
}

export function channelTooltipFooter(channel: Channel) {
  const details = [
    channel.visibility === "private" ? "Private channel" : "Public channel",
    channel.channelType === "forum" ? "Forum" : null,
    channel.archivedAt ? "Archived" : null,
    channel.lastMessageAt ? formatChannelActivity(channel.lastMessageAt) : null,
  ];
  return details.filter(Boolean).join(" · ");
}

function ChannelMetadataTooltip({
  channel,
  children,
}: {
  channel: Channel | undefined;
  children: React.ReactElement;
}) {
  const description = channel?.description?.trim();
  if (!channel) return children;
  return (
    <TooltipProvider delayDuration={500} skipDelayDuration={0}>
      <Tooltip>
        <TooltipTrigger asChild>{children}</TooltipTrigger>
        <TooltipContent
          align="start"
          className="max-w-72 p-2 text-left"
          side="top"
        >
          {description ? (
            <span
              className="line-clamp-2"
              data-buzz-tooltip-metadata-content=""
            >
              {description}
            </span>
          ) : null}
          <span
            className={cn(
              "block max-w-full truncate whitespace-nowrap text-2xs text-primary-foreground/70",
              description && "mt-1",
            )}
            data-buzz-tooltip-metadata-type=""
          >
            {channelTooltipFooter(channel)}
          </span>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

function channelPermalinkLabel(
  channels: ReturnType<typeof useMarkdownRuntime>["channels"],
  channelId: string,
): string {
  return (
    channels.find((candidate) => candidate.id === channelId)?.name ??
    channelId.slice(0, 8)
  );
}

export function ChannelDeepLinkAnchor({
  children,
  href,
  interactive,
}: React.ComponentPropsWithoutRef<"a"> & { interactive: boolean }) {
  const { channels, onOpenChannel, onOpenMessageLink } = useMarkdownRuntime();
  if (!href) return <>{children}</>;
  const parsed = parseChannelLink(href);
  if (!parsed.ok) return <>{children}</>;
  const messageLink = parsed.value.messageId
    ? {
        channelId: parsed.value.channelId,
        messageId: parsed.value.messageId,
        threadRootId: null,
      }
    : null;
  const openLink = () =>
    messageLink
      ? onOpenMessageLink(messageLink)
      : onOpenChannel(parsed.value.channelId);
  const authoredLabel = getReactNodeText(children);
  if (authoredLabel !== href) {
    return (
      <BuzzInlineLink
        href={href}
        title={href}
        aria-label={`${messageLink ? "Open message" : "Open channel"}: ${authoredLabel}`}
        interactive={interactive}
        onOpenLink={openLink}
      >
        {children}
      </BuzzInlineLink>
    );
  }
  if (messageLink) {
    return (
      <MessageLinkPill
        channels={channels}
        href={href}
        interactive={interactive}
        link={messageLink}
        onOpenChannel={onOpenChannel}
        onOpenMessageLink={onOpenMessageLink}
      />
    );
  }
  const label = channelPermalinkLabel(channels, parsed.value.channelId);
  const channel = channels.find(
    (candidate) => candidate.id === parsed.value.channelId,
  );
  return (
    <ChannelMetadataTooltip channel={channel}>
      <BuzzLinkChip
        href={href}
        icon="channel"
        aria-label={`Open channel ${label}`}
        interactive={interactive}
        onOpenLink={() => onOpenChannel(parsed.value.channelId)}
        wrapping
      >
        {label}
      </BuzzLinkChip>
    </ChannelMetadataTooltip>
  );
}

export function MarkdownChannelDeepLink({
  children,
  interactive,
}: {
  children?: React.ReactNode;
  interactive: boolean;
}) {
  const { channels, onOpenChannel, onOpenMessageLink } = useMarkdownRuntime();
  const href = String(children ?? "");
  const parsed = parseChannelLink(href);
  if (!parsed.ok) return <span data-channel-deep-link="">{href}</span>;
  const messageLink = parsed.value.messageId
    ? {
        channelId: parsed.value.channelId,
        messageId: parsed.value.messageId,
        threadRootId: null,
      }
    : null;
  if (messageLink) {
    return (
      <MessageLinkPill
        channels={channels}
        href={href}
        interactive={interactive}
        link={messageLink}
        onOpenChannel={onOpenChannel}
        onOpenMessageLink={onOpenMessageLink}
      />
    );
  }
  const label = channelPermalinkLabel(channels, parsed.value.channelId);
  const channel = channels.find(
    (candidate) => candidate.id === parsed.value.channelId,
  );
  return (
    <ChannelMetadataTooltip channel={channel}>
      <BuzzLinkChip
        data-channel-deep-link=""
        href={href}
        icon="channel"
        aria-label={`Open channel ${label}`}
        interactive={interactive}
        onOpenLink={() => onOpenChannel(parsed.value.channelId)}
        wrapping
      >
        {label}
      </BuzzLinkChip>
    </ChannelMetadataTooltip>
  );
}

export function MarkdownChannelReference({
  children,
  interactive,
}: {
  children?: React.ReactNode;
  interactive: boolean;
}) {
  const { channels, onOpenChannel } = useMarkdownRuntime();
  const text = String(children ?? "");
  const channelName = text.startsWith("#") ? text.slice(1) : text;
  const channel = channels.find(
    (candidate) =>
      candidate.channelType !== "dm" &&
      candidate.name.toLowerCase() === channelName.toLowerCase(),
  );
  return (
    <ChannelMetadataTooltip channel={channel}>
      <BuzzLinkChip
        data-channel-link=""
        href={channel ? buildChannelLink(channel.id) : undefined}
        icon="channel"
        aria-label={
          channel ? `Open channel ${channelName}` : `Channel ${channelName}`
        }
        interactive={Boolean(channel) && interactive}
        onOpenLink={() => {
          if (channel) onOpenChannel(channel.id);
        }}
        wrapping
      >
        {channelName}
      </BuzzLinkChip>
    </ChannelMetadataTooltip>
  );
}

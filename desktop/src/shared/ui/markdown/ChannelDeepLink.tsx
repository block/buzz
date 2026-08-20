import type * as React from "react";

import {
  isChannelReferenceOpenable,
  useChannelReference,
  useResolvedChannelDirectory,
} from "@/features/channels/openChannelDirectory";
import {
  buildChannelLink,
  parseChannelLink,
} from "@/features/messages/lib/channelLink";
import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

import { BuzzInlineLink, BuzzLinkChip } from "./BuzzLinkChip";
import { MessageLinkPill } from "./MessageLinkPill";
import { useMarkdownRuntime } from "./runtimeContext";
import { useInlineTooltipPosition } from "./useInlineTooltipPosition";
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
  const { contentRef, onPointerMove } = useInlineTooltipPosition();
  const description = channel?.description?.trim();
  if (!channel) return children;
  return (
    <TooltipProvider delayDuration={500} skipDelayDuration={0}>
      <Tooltip>
        <TooltipTrigger asChild onPointerMove={onPointerMove}>
          {children}
        </TooltipTrigger>
        <TooltipContent
          ref={contentRef}
          className="max-w-72 p-2 text-left"
          side="top"
        >
          {description ? (
            <span
              className="line-clamp-2 [overflow-wrap:anywhere] whitespace-normal"
              data-buzz-tooltip-metadata-content=""
            >
              {description}
            </span>
          ) : null}
          <span
            className={cn(
              "line-clamp-2 max-w-full [overflow-wrap:anywhere] whitespace-normal text-2xs text-secondary-foreground/80",
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

type ChannelPermalinkChipProps = {
  channelId: string;
  channels?: Channel[];
  dataChannelDeepLink?: string;
  href: string;
  interactive: boolean;
  onOpenChannel: (channelId: string) => void;
  resolveChannelReference?: boolean;
};

function ChannelPermalinkChipContents({
  channelId,
  dataChannelDeepLink,
  href,
  channel,
  interactive,
  label,
  onOpenChannel,
  openable = true,
}: Omit<ChannelPermalinkChipProps, "channels" | "resolveChannelReference"> & {
  channel?: Channel;
  label: string;
  openable?: boolean;
}) {
  return (
    <ChannelMetadataTooltip channel={channel}>
      <BuzzLinkChip
        data-channel-deep-link={dataChannelDeepLink}
        href={href}
        icon="channel"
        title={href}
        aria-label={openable ? `Open channel ${label}` : `Channel ${label}`}
        interactive={openable && interactive}
        onOpenLink={() => {
          if (openable) onOpenChannel(channelId);
        }}
        wrapping
      >
        {label}
      </BuzzLinkChip>
    </ChannelMetadataTooltip>
  );
}

function ResolvedChannelPermalinkChip(props: ChannelPermalinkChipProps) {
  const channel = useChannelReference(props.channelId);
  const openable = isChannelReferenceOpenable(channel);
  const label = openable ? channel.name : props.channelId.slice(0, 8);
  return (
    <ChannelPermalinkChipContents
      {...props}
      channel={openable ? channel : undefined}
      label={label}
      openable={openable}
    />
  );
}

type AuthoredMessageLink = ParsedMessageLink;

type AuthoredDeepLinkProps = {
  channelId: string;
  children: React.ReactNode;
  href: string;
  interactive: boolean;
  messageLink: AuthoredMessageLink | null;
  onOpenChannel: (channelId: string) => void;
  onOpenMessageLink: (link: AuthoredMessageLink) => void;
};

function ResolvedAuthoredDeepLink({
  channelId,
  children,
  href,
  interactive,
  messageLink,
  onOpenChannel,
  onOpenMessageLink,
}: AuthoredDeepLinkProps) {
  const channel = useChannelReference(channelId);
  const openable = isChannelReferenceOpenable(channel);
  const label = getReactNodeText(children);
  if (!openable) {
    return (
      <span className="font-medium text-current" data-buzz-link={href}>
        {children}
      </span>
    );
  }
  return (
    <BuzzInlineLink
      href={href}
      title={href}
      aria-label={`${messageLink ? "Open message" : "Open channel"}: ${label}`}
      interactive={interactive}
      onOpenLink={() =>
        messageLink ? onOpenMessageLink(messageLink) : onOpenChannel(channelId)
      }
    >
      {children}
    </BuzzInlineLink>
  );
}

/**
 * Renders an intentionally-labeled `buzz://channel|message` deep link through
 * the shared visibility gate. Known channels are interactive immediately;
 * unknown ids without a runtime resolver stay inert; otherwise a bounded
 * per-id lookup decides openability. Both parser families
 * (`buzz://channel/...` and `buzz://message?...`) share this decision so a
 * private destination can never render clickable via a custom label.
 */
export function AuthoredDeepLinkAnchor({
  channelId,
  children,
  href,
  interactive,
  messageLink,
}: {
  channelId: string;
  children: React.ReactNode;
  href: string;
  interactive: boolean;
  messageLink: AuthoredMessageLink | null;
}) {
  const {
    channels,
    onOpenChannel,
    onOpenMessageLink,
    resolveChannelReferences,
  } = useMarkdownRuntime();
  const openLink = () =>
    messageLink ? onOpenMessageLink(messageLink) : onOpenChannel(channelId);
  const label = getReactNodeText(children);
  const knownChannel = channels?.find((c) => c.id === channelId);
  if (knownChannel) {
    return (
      <BuzzInlineLink
        href={href}
        title={href}
        aria-label={`${messageLink ? "Open message" : "Open channel"}: ${label}`}
        interactive={interactive}
        onOpenLink={openLink}
      >
        {children}
      </BuzzInlineLink>
    );
  }
  if (!resolveChannelReferences) {
    return (
      <span className="font-medium text-current" data-buzz-link={href}>
        {children}
      </span>
    );
  }
  return (
    <ResolvedAuthoredDeepLink
      channelId={channelId}
      href={href}
      interactive={interactive}
      messageLink={messageLink}
      onOpenChannel={onOpenChannel}
      onOpenMessageLink={onOpenMessageLink}
    >
      {children}
    </ResolvedAuthoredDeepLink>
  );
}

/**
 * A permalink labels from its synchronous member list first. Only production
 * markdown requests the bounded detail lookup for an unknown id; static
 * renderers retain the short-id fallback without requiring app providers.
 */
function ChannelPermalinkChip(props: ChannelPermalinkChipProps) {
  const known = props.channels?.find(
    (channel) => channel.id === props.channelId,
  );
  if (known || !props.resolveChannelReference) {
    return (
      <ChannelPermalinkChipContents
        {...props}
        channel={known}
        label={known?.name ?? props.channelId.slice(0, 8)}
      />
    );
  }
  return <ResolvedChannelPermalinkChip {...props} />;
}

export function ChannelDeepLinkAnchor({
  children,
  href,
  interactive,
}: React.ComponentPropsWithoutRef<"a"> & { interactive: boolean }) {
  const {
    channels,
    onOpenChannel,
    onOpenMessageLink,
    resolveChannelReferences,
  } = useMarkdownRuntime();
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
  const authoredLabel = getReactNodeText(children);
  if (authoredLabel !== href) {
    return (
      <AuthoredDeepLinkAnchor
        channelId={parsed.value.channelId}
        href={href}
        interactive={interactive}
        messageLink={messageLink}
      >
        {children}
      </AuthoredDeepLinkAnchor>
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
        resolveChannelReference={resolveChannelReferences}
      />
    );
  }
  return (
    <ChannelPermalinkChip
      channelId={parsed.value.channelId}
      channels={channels}
      href={href}
      interactive={interactive}
      onOpenChannel={onOpenChannel}
      resolveChannelReference={resolveChannelReferences}
    />
  );
}

export function MarkdownChannelDeepLink({
  children,
  interactive,
}: {
  children?: React.ReactNode;
  interactive: boolean;
}) {
  const {
    channels,
    onOpenChannel,
    onOpenMessageLink,
    resolveChannelReferences,
  } = useMarkdownRuntime();
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
        resolveChannelReference={resolveChannelReferences}
      />
    );
  }
  return (
    <ChannelPermalinkChip
      channelId={parsed.value.channelId}
      channels={channels}
      dataChannelDeepLink=""
      href={href}
      interactive={interactive}
      onOpenChannel={onOpenChannel}
      resolveChannelReference={resolveChannelReferences}
    />
  );
}

type ChannelReferenceChipProps = {
  channelName: string;
  channel?: Channel;
  interactive: boolean;
  onOpenChannel: (channelId: string) => void;
};

function ChannelReferenceChip({
  channel,
  channelName,
  interactive,
  onOpenChannel,
}: ChannelReferenceChipProps) {
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

function ResolvedChannelReferenceChip({
  channelName,
  interactive,
  onOpenChannel,
}: Omit<ChannelReferenceChipProps, "channel">) {
  const { channels } = useResolvedChannelDirectory();
  const channel = channels.find(
    (candidate) =>
      candidate.channelType !== "dm" &&
      candidate.name.toLowerCase() === channelName.toLowerCase(),
  );
  const openable = isChannelReferenceOpenable(channel);
  return (
    <ChannelReferenceChip
      channel={openable ? channel : undefined}
      channelName={channelName}
      interactive={interactive}
      onOpenChannel={onOpenChannel}
    />
  );
}

export function MarkdownChannelReference({
  children,
  interactive,
}: {
  children?: React.ReactNode;
  interactive: boolean;
}) {
  const { channels, onOpenChannel, resolveChannelReferences } =
    useMarkdownRuntime();
  const text = String(children ?? "");
  const channelName = text.startsWith("#") ? text.slice(1) : text;
  const known = channels.find(
    (candidate) =>
      candidate.channelType !== "dm" &&
      candidate.name.toLowerCase() === channelName.toLowerCase(),
  );

  if (known || !resolveChannelReferences) {
    return (
      <ChannelReferenceChip
        channel={known}
        channelName={channelName}
        interactive={interactive}
        onOpenChannel={onOpenChannel}
      />
    );
  }

  // Name → channel has no relay index. This observes only a browse/search-warm
  // directory; a cold non-member name remains an inert chip by construction.
  return (
    <ResolvedChannelReferenceChip
      channelName={channelName}
      interactive={interactive}
      onOpenChannel={onOpenChannel}
    />
  );
}

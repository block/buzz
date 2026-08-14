import type * as React from "react";

import {
  buildChannelLink,
  parseChannelLink,
} from "@/features/messages/lib/channelLink";

import { BuzzInlineLink, BuzzLinkChip } from "./BuzzLinkChip";
import { useMarkdownRuntime } from "./runtimeContext";
import { getReactNodeText } from "./utils";

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
  const { channels, onOpenChannel } = useMarkdownRuntime();
  if (!href) return <>{children}</>;
  const parsed = parseChannelLink(href);
  if (!parsed.ok) return <>{children}</>;
  const authoredLabel = getReactNodeText(children);
  if (authoredLabel !== href) {
    return (
      <BuzzInlineLink
        href={href}
        title={href}
        aria-label={`Open channel: ${authoredLabel}`}
        interactive={interactive}
        onOpenLink={() => onOpenChannel(parsed.value.channelId)}
      >
        {children}
      </BuzzInlineLink>
    );
  }
  const label = channelPermalinkLabel(channels, parsed.value.channelId);
  return (
    <BuzzLinkChip
      href={href}
      icon="channel"
      title={href}
      aria-label={`Open channel ${label}`}
      interactive={interactive}
      onOpenLink={() => onOpenChannel(parsed.value.channelId)}
    >
      {label}
    </BuzzLinkChip>
  );
}

export function MarkdownChannelDeepLink({
  children,
  interactive,
}: {
  children?: React.ReactNode;
  interactive: boolean;
}) {
  const { channels, onOpenChannel } = useMarkdownRuntime();
  const href = String(children ?? "");
  const parsed = parseChannelLink(href);
  if (!parsed.ok) return <span data-channel-deep-link="">{href}</span>;
  const label = channelPermalinkLabel(channels, parsed.value.channelId);
  return (
    <BuzzLinkChip
      data-channel-deep-link=""
      href={href}
      icon="channel"
      title={href}
      aria-label={`Open channel ${label}`}
      interactive={interactive}
      onOpenLink={() => onOpenChannel(parsed.value.channelId)}
    >
      {label}
    </BuzzLinkChip>
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
    >
      {channelName}
    </BuzzLinkChip>
  );
}

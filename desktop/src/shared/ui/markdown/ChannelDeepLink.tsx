import type * as React from "react";

import { parseChannelLink } from "@/features/messages/lib/channelLink";

import { useMarkdownRuntime } from "./runtimeContext";

const CHANNEL_LINK_CLASSES =
  "font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80 cursor-pointer";

export function ChannelDeepLinkAnchor({
  children,
  href,
  ...props
}: React.ComponentPropsWithoutRef<"a">) {
  const { onOpenChannel } = useMarkdownRuntime();
  if (!href) return <>{children}</>;
  const parsed = parseChannelLink(href);
  if (!parsed.ok) return <>{children}</>;
  return (
    <a
      {...props}
      className={CHANNEL_LINK_CLASSES}
      href={href}
      onClick={(event) => {
        event.preventDefault();
        onOpenChannel(parsed.value.channelId);
      }}
    >
      {children}
    </a>
  );
}

export function MarkdownChannelDeepLink({
  children,
  interactive,
}: {
  children?: React.ReactNode;
  interactive: boolean;
}) {
  const { onOpenChannel } = useMarkdownRuntime();
  const href = String(children ?? "");
  const parsed = parseChannelLink(href);
  if (!parsed.ok || !interactive) {
    return <span data-channel-deep-link="">{href}</span>;
  }
  return (
    <button
      type="button"
      data-channel-deep-link=""
      className={CHANNEL_LINK_CLASSES}
      onClick={() => onOpenChannel(parsed.value.channelId)}
    >
      {href}
    </button>
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
  const baseClasses =
    "inline-flex items-center rounded-md bg-primary/10 px-1 py-0.5 font-medium text-primary";
  if (!channel || !interactive) {
    return (
      <span data-channel-link="" className={baseClasses}>
        {children}
      </span>
    );
  }
  return (
    <button
      type="button"
      data-channel-link=""
      aria-label={`Open channel ${channelName}`}
      className={`${baseClasses} cursor-pointer hover:bg-primary/20`}
      onClick={() => onOpenChannel(channel.id)}
    >
      {children}
    </button>
  );
}

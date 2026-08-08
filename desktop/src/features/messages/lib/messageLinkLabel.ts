export type MessageLinkLabelVariant = "default" | "sent-from-thread";

export const MESSAGE_LINK_PREFIX = "Thread in";

export function getMessageLinkChannelLabel(channelName: string): string {
  return `#${channelName}`;
}

export function getMessageLinkLabel({
  channelName,
  threadExcerpt,
  variant = "default",
}: {
  channelName: string;
  threadExcerpt?: string | null;
  variant?: MessageLinkLabelVariant;
}): string {
  const normalizedExcerpt = threadExcerpt?.trim();
  const baseLabel = `${MESSAGE_LINK_PREFIX} ${getMessageLinkChannelLabel(channelName)}`;
  if (variant === "sent-from-thread") {
    return normalizedExcerpt ?? baseLabel;
  }
  return normalizedExcerpt ? `${baseLabel} — ${normalizedExcerpt}` : baseLabel;
}

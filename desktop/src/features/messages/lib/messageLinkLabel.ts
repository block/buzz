import { i18n } from "@/i18n";

export type MessageLinkLabelVariant = "default" | "sent-from-thread";

export function getMessageLinkPrefix(): string {
  return i18n.t("messages.link.thread-in-prefix");
}

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
  const baseLabel = `${getMessageLinkPrefix()} ${getMessageLinkChannelLabel(channelName)}`;
  if (variant === "sent-from-thread") {
    return normalizedExcerpt ?? baseLabel;
  }
  return normalizedExcerpt ? `${baseLabel} — ${normalizedExcerpt}` : baseLabel;
}

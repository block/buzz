import { i18n } from "@/i18n";
import type { Channel } from "@/shared/api/types";

/** The authored channel detail shown consistently across channel surfaces. */
export function getChannelDetail(channel: Channel): string | null {
  return (
    [channel.topic, channel.description, channel.purpose]
      .find((value) => value && value.trim().length > 0)
      ?.trim() ?? null
  );
}

export function getChannelDescription(channel: Channel | null): string {
  if (!channel) {
    return i18n.t("channels.description.no-relay");
  }

  const prefixes = [
    channel.archivedAt ? i18n.t("channels.description.archived") : null,
    !channel.isMember
      ? i18n.t("channels.description.read-only-until-join")
      : null,
  ].filter((value) => value && value.trim().length > 0);

  // Show only the first non-empty field to avoid duplication when
  // topic, description, and purpose contain overlapping text.
  const detail = getChannelDetail(channel);

  // Join the status prefixes with spaces, but keep the detail text's own
  // line breaks intact (native `title` tooltips render newlines) and separate
  // it from the prefixes with a newline so paragraphs stay readable.
  const prefixText = prefixes.join(" ");
  const parts = [prefixText || null, detail ?? null].filter(Boolean);

  return parts.length > 0
    ? parts.join("\n")
    : i18n.t("channels.description.details");
}

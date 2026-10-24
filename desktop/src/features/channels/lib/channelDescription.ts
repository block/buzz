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
    return "Connect to the relay to browse channels and read messages.";
  }

  const prefixes = [
    channel.archivedAt ? "Archived." : null,
    !channel.isMember ? "Read-only until you join this open channel." : null,
  ].filter((value) => value && value.trim().length > 0);

  // Show only the first non-empty field to avoid duplication when
  // topic, description, and purpose contain overlapping text.
  const detail = getChannelDetail(channel);

  // Join the status prefixes with spaces, but keep the detail text's own
  // line breaks intact (native `title` tooltips render newlines) and separate
  // it from the prefixes with a newline so paragraphs stay readable.
  const prefixText = prefixes.join(" ");
  const parts = [prefixText || null, detail ?? null].filter(Boolean);

  return parts.length > 0 ? parts.join("\n") : "Channel details and activity.";
}

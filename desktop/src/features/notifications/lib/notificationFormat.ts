// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
const NOTIFICATION_BODY_MAX_LENGTH = 140;

/**
 * Resolve a channel's display label for use in notification titles.
 *
 * Returns `"#channelName"` when the channel is found and has a non-empty name,
 * or `null` when the channelId is absent or the channel is not yet in the list
 * (e.g. channels query hasn't resolved — the caller should fall back gracefully
 * rather than blocking the toast).
 */
export function resolveNotificationChannelLabel(
  channelId: string | null | undefined,
  channels: ReadonlyArray<{ id: string; name?: string | null }>,
): string | null {
  if (!channelId) return null;
  const channel = channels.find((c) => c.id === channelId);
  const name = channel?.name?.trim();
  return name ? `#${name}` : null;
}

/**
 * Truncate notification body text to {@link NOTIFICATION_BODY_MAX_LENGTH}
 * characters, appending "..." when truncated.  Returns `fallback` when
 * `content` is blank after trimming.
 */
export function truncateNotificationBody(
  content: string,
  fallback: string,
): string {
  const trimmed = content.trim();
  if (trimmed.length === 0) return fallback;
  if (trimmed.length <= NOTIFICATION_BODY_MAX_LENGTH) return trimmed;
  return `${trimmed.slice(0, NOTIFICATION_BODY_MAX_LENGTH - 3).trimEnd()}...`;
}

/**
 * Format a notification title with optional channel context.
 *
 * - With a channel label: `"prefix in #channel"`
 * - Without: `"prefix"`
 */
export function formatNotificationTitle(opts: {
  prefix: string;
  channelLabel: string | null;
}): string {
  return opts.channelLabel
    ? uiT("{0} in {1}", { p0: opts.prefix, p1: opts.channelLabel })
    : opts.prefix;
}

export type MessageNotificationSource =
  | "mention"
  | "approval"
  | "needs_action"
  | "dm"
  | "thread_reply";

const MESSAGE_BODY_FALLBACKS: Record<MessageNotificationSource, string> = {
  get mention() {
    return uiT("Something in Buzz needs your attention.");
  },
  get approval() {
    return uiT("A workflow is waiting for your approval.");
  },
  get needs_action() {
    return uiT("Something in Buzz needs your attention.");
  },
  get dm() {
    return uiT("New message");
  },
  get thread_reply() {
    return uiT("New reply");
  },
};

/**
 * Canonical copy for every message-shaped desktop notification (home-feed
 * mentions and needs-action items, live DMs, live thread replies). All paths
 * format through here so sender attribution and fallbacks stay consistent:
 * the sender leads the title whenever their profile has resolved, and each
 * source degrades to neutral copy — never a raw pubkey — when it has not.
 *
 * `senderName` must already be a real human label (see
 * `senderNameFromSummary`); `channelName` is the raw channel name without
 * a `#` prefix.
 */
export function formatMessageNotification(opts: {
  source: MessageNotificationSource;
  senderName?: string | null;
  channelName?: string | null;
  content: string;
}): { title: string; body: string } {
  const { source, content } = opts;
  const senderName = opts.senderName?.trim() || null;
  const channelName = opts.channelName?.trim() || null;
  const body = truncateNotificationBody(
    content,
    MESSAGE_BODY_FALLBACKS[source],
  );

  if (source === "dm") {
    return { title: senderName ?? channelName ?? uiT("Direct message"), body };
  }

  const channelLabel = channelName ? `#${channelName}` : null;
  const prefix =
    source === "mention"
      ? senderName
        ? uiT("{0} mentioned you", { p0: senderName })
        : uiT("@Mention")
      : source === "approval"
        ? senderName
          ? uiT("{0} requested approval", { p0: senderName })
          : uiT("Approval Requested")
        : source === "thread_reply"
          ? senderName
            ? uiT("{0} replied", { p0: senderName })
            : uiT("Reply")
          : (senderName ?? uiT("Needs Action"));

  return { title: formatNotificationTitle({ prefix, channelLabel }), body };
}

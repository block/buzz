/** `buzz://channel/<uuid>[/<event-id>]` link encoding and parsing. */

const CHANNEL_LINK_SCHEME = "buzz:";
const CHANNEL_LINK_HOST = "channel";
const CHANNEL_UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu;
const EVENT_ID_PATTERN = /^[0-9a-f]{64}$/iu;

export type ParsedChannelLink = {
  channelId: string;
  messageId?: string;
};

export type ChannelLinkParseResult =
  | { ok: true; value: ParsedChannelLink }
  | { ok: false; reason: string };

export function buildChannelLink(channelId: string): string {
  if (!channelId) {
    throw new Error("buildChannelLink: channelId is required");
  }
  return `${CHANNEL_LINK_SCHEME}//${CHANNEL_LINK_HOST}/${encodeURIComponent(channelId)}`;
}

export function parseChannelLink(url: string): ChannelLinkParseResult {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }
  if (parsed.protocol !== CHANNEL_LINK_SCHEME) {
    return { ok: false, reason: "wrong-scheme" };
  }
  if (parsed.hostname !== CHANNEL_LINK_HOST) {
    return { ok: false, reason: "wrong-host" };
  }
  if (parsed.search || parsed.hash || parsed.username || parsed.password) {
    return { ok: false, reason: "unexpected-components" };
  }
  const segments = parsed.pathname.split("/").slice(1);
  if (segments.length < 1 || segments.length > 2 || segments.includes("")) {
    return { ok: false, reason: "missing-or-extra-channel" };
  }
  let channelId: string;
  let messageId: string | null = null;
  try {
    channelId = decodeURIComponent(segments[0]);
    messageId = segments[1] ? decodeURIComponent(segments[1]) : null;
  } catch {
    return { ok: false, reason: "invalid-channel-encoding" };
  }
  if (!CHANNEL_UUID_PATTERN.test(channelId)) {
    return { ok: false, reason: "invalid-channel-uuid" };
  }
  if (messageId !== null && !EVENT_ID_PATTERN.test(messageId)) {
    return { ok: false, reason: "invalid-message-id" };
  }
  return {
    ok: true,
    value: {
      channelId: channelId.toLowerCase(),
      ...(messageId ? { messageId: messageId.toLowerCase() } : {}),
    },
  };
}

export function isChannelLink(href: string | undefined | null): boolean {
  return href ? parseChannelLink(href).ok : false;
}

/**
 * `sprout://message` link encoding for "Copy link" / deep-link-to-message.
 *
 * Format: `sprout://message?channel=<uuid>&id=<eventId>[&thread=<rootId>]`
 *
 * Mirrors the existing `sprout://connect?relay=…` scheme already registered
 * in `tauri.conf.json` and handled in `desktop/src-tauri/src/lib.rs`.
 */

const MESSAGE_LINK_HOST = "message";

export type MessageLinkInput = {
  channelId: string;
  messageId: string;
  /**
   * Optional thread root event id. Present when the linked message is a
   * reply (so the caller can route into a thread / forum post view).
   *
   * Currently emitted into the URL but not consumed by the click handler
   * or deep-link listener — both route via `goChannel(channelId,
   * { messageId })` and let `useTimelineScrollManager` resolve the target.
   * Reserved for future "open in thread view" routing.
   */
  threadRootId?: string | null;
};

export type ParsedMessageLink = {
  channelId: string;
  messageId: string;
  threadRootId: string | null;
};

export type MessageLinkParseResult =
  | { ok: true; value: ParsedMessageLink }
  | { ok: false; reason: string };

/**
 * Build a `sprout://message` URL for a given channel + message.
 *
 * Empty `threadRootId` is treated as "no thread" so callers can pass through
 * the result of `getThreadReference(tags).rootId` without extra null checks.
 */
export function buildMessageLink(input: MessageLinkInput): string {
  if (!input.channelId) {
    throw new Error("buildMessageLink: channelId is required");
  }
  if (!input.messageId) {
    throw new Error("buildMessageLink: messageId is required");
  }

  const params = new URLSearchParams();
  params.set("channel", input.channelId);
  params.set("id", input.messageId);
  if (input.threadRootId) {
    params.set("thread", input.threadRootId);
  }
  return `sprout://${MESSAGE_LINK_HOST}?${params.toString()}`;
}

/**
 * Parse a `sprout://message?…` URL. Returns a discriminated result so
 * callers can render a fallback (e.g. a plain link) without throwing.
 */
export function parseMessageLink(url: string): MessageLinkParseResult {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }

  if (parsed.protocol !== "sprout:") {
    return { ok: false, reason: "wrong-scheme" };
  }
  // `new URL("sprout://message?…")` puts "message" in `hostname`.
  if (parsed.hostname !== MESSAGE_LINK_HOST) {
    return { ok: false, reason: "wrong-host" };
  }

  const channelId = parsed.searchParams.get("channel");
  const messageId = parsed.searchParams.get("id");
  if (!channelId) {
    return { ok: false, reason: "missing-channel" };
  }
  if (!messageId) {
    return { ok: false, reason: "missing-id" };
  }

  return {
    ok: true,
    value: {
      channelId,
      messageId,
      threadRootId: parsed.searchParams.get("thread") ?? null,
    },
  };
}

/**
 * Convenience: returns true if the given href is a `sprout://message` link.
 * Cheap pre-check used by the markdown renderer before parsing.
 */
export function isMessageLink(href: string | undefined | null): boolean {
  if (!href) return false;
  return href.startsWith("sprout://message?") || href === "sprout://message";
}

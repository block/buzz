/**
 * `buzz://chat` link encoding and route parsing for hidden chat channels.
 *
 * Format: `buzz://chat?channel=<chat-channel-id>[&title=<display-title>]`
 * Route form: `/chats/<chat-channel-id>[?title=<display-title>]`
 */

const CHAT_LINK_SCHEME = "buzz:";
const CHAT_LINK_HOST = "chat";

export type ChatLinkInput = {
  chatId: string;
  title?: string | null;
};

type ParseChatRouteLinkOptions = {
  currentOrigin?: string | null;
};

export type ParsedChatLink = {
  chatId: string;
  title: string | null;
};

export type ChatLinkParseResult =
  | { ok: true; value: ParsedChatLink }
  | { ok: false; reason: string };

export function buildChatLink(input: ChatLinkInput): string {
  if (!input.chatId) {
    throw new Error("buildChatLink: chatId is required");
  }

  const params = new URLSearchParams();
  params.set("channel", input.chatId);
  const title = input.title?.trim();
  if (title) {
    params.set("title", title);
  }
  return `${CHAT_LINK_SCHEME}//${CHAT_LINK_HOST}?${params.toString()}`;
}

export function parseChatLink(url: string): ChatLinkParseResult {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }

  if (parsed.protocol !== CHAT_LINK_SCHEME) {
    return { ok: false, reason: "wrong-scheme" };
  }
  if (parsed.hostname !== CHAT_LINK_HOST) {
    return { ok: false, reason: "wrong-host" };
  }

  const chatId = parsed.searchParams.get("channel");
  if (!chatId) {
    return { ok: false, reason: "missing-channel" };
  }

  return {
    ok: true,
    value: {
      chatId,
      title: parsed.searchParams.get("title")?.trim() || null,
    },
  };
}

export function parseChatRouteLink(
  url: string,
  options: ParseChatRouteLinkOptions = {},
): ChatLinkParseResult {
  let parsed: URL;
  const currentOrigin = options.currentOrigin ?? null;
  const fallbackOrigin = currentOrigin || "https://buzz.local";

  try {
    parsed = new URL(url, fallbackOrigin);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }

  const isAbsolute = /^[a-z][a-z0-9+.-]*:/i.test(url);
  if (isAbsolute && (!currentOrigin || parsed.origin !== currentOrigin)) {
    return { ok: false, reason: "wrong-origin" };
  }

  const match = parsed.pathname.match(/^\/chats\/([^/?#]+)\/?$/);
  if (!match?.[1]) {
    return { ok: false, reason: "wrong-path" };
  }

  return {
    ok: true,
    value: {
      chatId: decodeURIComponent(match[1]),
      title: parsed.searchParams.get("title")?.trim() || null,
    },
  };
}

export function isChatLink(href: string | undefined | null): boolean {
  if (!href) return false;
  return href.startsWith("buzz://chat?") || href === "buzz://chat";
}

type ChatLinkRenderInput = {
  href: string;
  label: string;
};

export type ChatLinkRenderTarget =
  | { kind: "card"; link: ParsedChatLink }
  | { kind: "label"; link: ParsedChatLink }
  | { kind: "none" };

export function resolveChatLinkRenderTarget({
  href,
  label,
}: ChatLinkRenderInput): ChatLinkRenderTarget {
  const parsed = isChatLink(href)
    ? parseChatLink(href)
    : parseChatRouteLink(href, {
        currentOrigin:
          typeof window !== "undefined" ? window.location.origin : null,
      });
  if (!parsed.ok) return { kind: "none" };

  return {
    kind: label === href ? "card" : "label",
    link: parsed.value,
  };
}

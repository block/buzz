const AGENT_CONVERSATION_LINK_SCHEME = "buzz:";
const AGENT_CONVERSATION_LINK_HOST = "task";
export const AGENT_CONVERSATION_LINK_URL_PATTERN =
  /buzz:\/\/task\?[^\s<>"')\]]+/g;
const TRAILING_PUNCTUATION_PATTERN = /[.,;:!?]+$/;

export type AgentConversationLinkInput = {
  agentReplyId: string;
  channelId: string;
};

export type ParsedAgentConversationLink = {
  agentReplyId: string;
  channelId: string;
};

export type AgentConversationLinkParseResult =
  | { ok: true; value: ParsedAgentConversationLink }
  | { ok: false; reason: string };

export function buildAgentConversationLink(
  input: AgentConversationLinkInput,
): string {
  if (!input.channelId) {
    throw new Error("buildAgentConversationLink: channelId is required");
  }
  if (!input.agentReplyId) {
    throw new Error("buildAgentConversationLink: agentReplyId is required");
  }

  const params = new URLSearchParams();
  params.set("channel", input.channelId);
  params.set("reply", input.agentReplyId);

  return `${AGENT_CONVERSATION_LINK_SCHEME}//${AGENT_CONVERSATION_LINK_HOST}?${params.toString()}`;
}

export function parseAgentConversationLink(
  url: string,
): AgentConversationLinkParseResult {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }

  if (parsed.protocol !== AGENT_CONVERSATION_LINK_SCHEME) {
    return { ok: false, reason: "wrong-scheme" };
  }
  if (parsed.hostname !== AGENT_CONVERSATION_LINK_HOST) {
    return { ok: false, reason: "wrong-host" };
  }

  const channelId = parsed.searchParams.get("channel");
  const agentReplyId = parsed.searchParams.get("reply");
  if (!channelId) {
    return { ok: false, reason: "missing-channel" };
  }
  if (!agentReplyId) {
    return { ok: false, reason: "missing-reply" };
  }

  return {
    ok: true,
    value: {
      agentReplyId,
      channelId,
    },
  };
}

export function isAgentConversationLink(
  href: string | undefined | null,
): boolean {
  if (!href) return false;
  return (
    href.startsWith(
      `${AGENT_CONVERSATION_LINK_SCHEME}//${AGENT_CONVERSATION_LINK_HOST}?`,
    ) ||
    href ===
      `${AGENT_CONVERSATION_LINK_SCHEME}//${AGENT_CONVERSATION_LINK_HOST}`
  );
}

function isUnmatchedClosing(value: string): boolean {
  const closing = value[value.length - 1];
  const opening = closing === ")" ? "(" : "[";
  return value.split(closing).length > value.split(opening).length;
}

export function trimAgentConversationLinkMatch(matchText: string) {
  let value = matchText.replace(TRAILING_PUNCTUATION_PATTERN, "");
  while (/[)\]]$/.test(value) && isUnmatchedClosing(value)) {
    value = value.slice(0, -1).replace(TRAILING_PUNCTUATION_PATTERN, "");
  }
  return { value, trailing: matchText.slice(value.length) };
}

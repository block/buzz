const AGENT_CONVERSATION_LINK_SCHEME = "buzz:";
const AGENT_CONVERSATION_LINK_HOST = "task";

export type AgentConversationLinkInput = {
  agentReplyId: string;
  channelId: string;
};

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

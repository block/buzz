import {
  AGENT_CONVERSATION_LINK_URL_PATTERN,
  isAgentConversationLink,
  trimAgentConversationLinkMatch,
} from "@/features/agents/agentConversationLink";
import { AGENT_CONVERSATION_LINK_NODE_NAME } from "@/features/messages/lib/agentConversationLinkNodeName";

type TaskLinkPasteContent =
  | { type: "text"; text: string }
  | {
      type: typeof AGENT_CONVERSATION_LINK_NODE_NAME;
      attrs: { href: string; title: string };
    };

export function buildTaskLinkPasteContent(
  text: string,
  titleForHref?: (href: string) => string | undefined,
): TaskLinkPasteContent[] | null {
  AGENT_CONVERSATION_LINK_URL_PATTERN.lastIndex = 0;
  const content: TaskLinkPasteContent[] = [];
  let cursor = 0;
  let hasTaskLink = false;

  for (const match of text.matchAll(AGENT_CONVERSATION_LINK_URL_PATTERN)) {
    const matchText = match[0];
    const matchIndex = match.index ?? 0;
    const { value, trailing } = trimAgentConversationLinkMatch(matchText);
    if (!isAgentConversationLink(value)) {
      continue;
    }

    if (matchIndex > cursor) {
      content.push({ type: "text", text: text.slice(cursor, matchIndex) });
    }

    content.push({
      type: AGENT_CONVERSATION_LINK_NODE_NAME,
      attrs: {
        href: value,
        title: titleForHref?.(value) ?? "",
      },
    });
    if (trailing) {
      content.push({ type: "text", text: trailing });
    }

    cursor = matchIndex + matchText.length;
    hasTaskLink = true;
  }

  if (!hasTaskLink) {
    return null;
  }

  if (cursor < text.length) {
    content.push({ type: "text", text: text.slice(cursor) });
  }
  return content;
}

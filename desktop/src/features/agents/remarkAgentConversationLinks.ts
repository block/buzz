/**
 * Remark plugin that detects bare `buzz://task?…` URLs and replaces each with
 * a custom task-link element. The renderer turns that element into an in-app
 * task link instead of leaving a raw custom-scheme URL as inert text.
 */

import { createRemarkPrefixPlugin } from "../../shared/lib/createRemarkPrefixPlugin.ts";
import {
  AGENT_CONVERSATION_LINK_URL_PATTERN,
  trimAgentConversationLinkMatch,
} from "./agentConversationLink.ts";

export default function remarkAgentConversationLinks() {
  return createRemarkPrefixPlugin(
    AGENT_CONVERSATION_LINK_URL_PATTERN,
    (matchText) => {
      const { value, trailing } = trimAgentConversationLinkMatch(matchText);

      return {
        node: {
          type: "agent-conversation-link",
          value,
          data: {
            hName: "agent-conversation-link",
            hChildren: [{ type: "text", value }],
          },
        },
        trailing,
      };
    },
  );
}

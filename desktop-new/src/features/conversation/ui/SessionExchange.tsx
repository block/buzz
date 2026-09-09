import { AgentWorkArea } from "@/features/agent-activity/ui/AgentWorkArea";
import type { AgentWorkPresentation } from "@/features/agent-activity/ui/workPresentation";
import {
  ConversationMessage,
  type ConversationMessagePresentation,
} from "./ConversationMessage";

/** One prompt, stable peer-agent areas, then shared answers in supplied order. */
export function SessionExchange({
  prompt,
  work,
  replies,
}: {
  prompt: ConversationMessagePresentation;
  /** Ordered by the activity owner; never sorted by latest activity in the view. */
  work: readonly AgentWorkPresentation[];
  replies: readonly ConversationMessagePresentation[];
}) {
  return (
    <section className="session-exchange" aria-label="Session exchange">
      <ConversationMessage message={prompt} />
      <div className="session-exchange-work">
        {work.map((area) => (
          <AgentWorkArea key={area.id} work={area} />
        ))}
      </div>
      {replies.map((message) => (
        <ConversationMessage key={message.id} message={message} />
      ))}
    </section>
  );
}

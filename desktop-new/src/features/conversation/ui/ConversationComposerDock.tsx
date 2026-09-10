import type { AgentTurn } from "@/features/agent-activity/types";
import { AgentActivityRail } from "@/features/agent-activity/ui/AgentActivityRail";
import { MessageComposer } from "@/features/composer/ui/MessageComposer";

/** Content-sized authoring dock. Activity fades into reserved space without resizing the conversation. */
export function ConversationComposerDock({
  draft,
  onDraftChange,
  onSend,
  turns,
  placeholder,
  responseControl,
  autoFocus,
  recipients,
  onDelivered,
}: {
  onDelivered?: (content: string) => void;
  recipients?: readonly { pubkey: string; name: string; isAgent: boolean }[];
  autoFocus?: boolean;
  draft: string;
  onDraftChange: (value: string) => void;
  onSend: (content: string, recipients?: string[]) => Promise<void>;
  turns: AgentTurn[];
  placeholder?: string;
  responseControl?: {
    state: "responding" | "stopping";
    onStop?: () => void;
  };
}) {
  const workingTurns = turns.filter(
    (turn) => turn.status === "pending" || turn.status === "running",
  );

  return (
    <div
      className="conversation-composer-dock"
      data-activity={workingTurns.length > 0 || undefined}
    >
      <MessageComposer
        autoFocus={autoFocus}
        recipients={recipients}
        onDelivered={onDelivered}
        draft={draft}
        onDraftChange={onDraftChange}
        onSend={onSend}
        placeholder={placeholder}
        responseControl={responseControl}
      />
      <div className="conversation-composer-activity-slot text-body-sm">
        <AgentActivityRail turns={workingTurns} />
      </div>
    </div>
  );
}

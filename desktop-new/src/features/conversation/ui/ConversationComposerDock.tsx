import type { AgentTurn } from "@/features/agent-activity/types";
import { AgentActivityRail } from "@/features/agent-activity/ui/AgentActivityRail";
import { MessageComposer } from "@/features/composer/ui/MessageComposer";

/**
 * The conversation's fixed-height bottom dock.
 *
 * Its grid owns the spatial relationship between authoring and activity. When
 * activity arrives, it takes its natural row below the Composer; the Composer
 * yields only from its lower edge while its top stays fixed.
 */
export function ConversationComposerDock({
  draft,
  onDraftChange,
  onSend,
  turns,
  placeholder,
  responseControl,
}: {
  draft: string;
  onDraftChange: (value: string) => void;
  onSend: (content: string) => Promise<void>;
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
        draft={draft}
        onDraftChange={onDraftChange}
        onSend={onSend}
        placeholder={placeholder}
        responseControl={responseControl}
      />
      <AgentActivityRail turns={workingTurns} />
    </div>
  );
}

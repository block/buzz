import { IconLoader2 } from "@tabler/icons-react";

import { latestActivityItem } from "@/features/agent-activity/activityProjection";
import type { AgentTurn } from "@/features/agent-activity/types";
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
}: {
  draft: string;
  onDraftChange: (value: string) => void;
  onSend: (content: string) => Promise<void>;
  turns: AgentTurn[];
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
      />
      {workingTurns.length > 0 ? (
        <div className="conversation-activity-rail" role="status">
          {workingTurns.map((turn) => {
            const current = latestActivityItem(turn);
            return (
              <div className="conversation-activity-rail-item" key={turn.key}>
                <IconLoader2
                  className="animate-spin"
                  size={15}
                  stroke={1.7}
                  aria-hidden="true"
                />
                <span className="min-w-0 text-body-sm text-secondary">
                  <span className="font-semibold text-primary">
                    {turn.agentName}
                  </span>{" "}
                  is working{current ? ` · ${current.label}` : ""}
                </span>
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

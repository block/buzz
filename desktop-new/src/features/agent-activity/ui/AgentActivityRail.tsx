import { latestActivityItem } from "../activityProjection";
import type { AgentTurn } from "../types";

/**
 * Compact, non-blocking status beneath a Conversation Composer.
 *
 * It renders only active turns. ConversationDock owns its space; this component
 * owns the activity's readable, motion-safe working treatment.
 */
export function AgentActivityRail({ turns }: { turns: AgentTurn[] }) {
  if (turns.length === 0) return null;

  return (
    <div className="agent-activity-rail" role="status">
      {turns.map((turn) => {
        const current = latestActivityItem(turn);
        const label = `${turn.agentName} is working${
          current ? ` · ${current.label}` : ""
        }`;
        return (
          <span
            key={turn.key}
            className="agent-activity-rail-status text-body-sm"
            aria-label={label}
          >
            {label}
          </span>
        );
      })}
    </div>
  );
}

import { Accordion } from "@base-ui/react/accordion";
import {
  IconAlertCircle,
  IconCheck,
  IconChevronDown,
  IconLoader2,
  IconPlayerPause,
} from "@tabler/icons-react";
import { partitionActivity } from "../activityProjection";
import type { ActivityItem, AgentTurn } from "../types";

function ActivityIcon({ item }: { item: ActivityItem }) {
  const common = { size: 15, stroke: 1.7, "aria-hidden": true } as const;
  if (item.status === "completed") return <IconCheck {...common} />;
  if (item.status === "failed" || item.status === "needs_you") {
    return <IconAlertCircle {...common} />;
  }
  if (item.status === "cancelled" || item.status === "unavailable") {
    return <IconPlayerPause {...common} />;
  }
  return <IconLoader2 {...common} className="animate-spin" />;
}

function ActivityRows({ items }: { items: ActivityItem[] }) {
  return (
    <ol className="activity-rows">
      {items.map((item) => (
        <li key={item.id} data-status={item.status}>
          <span className="activity-step-icon">
            <ActivityIcon item={item} />
          </span>
          <span className="min-w-0">
            <span className="block text-body text-primary">{item.label}</span>
            {item.detail ? (
              <span className="block truncate text-body-sm text-tertiary">
                {item.detail}
              </span>
            ) : null}
          </span>
        </li>
      ))}
    </ol>
  );
}

export function AgentActivity({ turn }: { turn: AgentTurn }) {
  const { visibleItems, hiddenItems } = partitionActivity(turn);
  const hiddenCount = hiddenItems.length;
  const expandedLabel = `${turn.agentName} activity, ${turn.items.length} steps`;

  return (
    <section className="agent-turn" aria-label={`${turn.agentName} activity`}>
      <div className="agent-turn-heading">
        <span className="agent-mark" aria-hidden="true">
          {turn.agentName.slice(0, 1)}
        </span>
        <span className="text-body text-primary">{turn.agentName}</span>
        <span className="text-body-sm text-tertiary">
          {turn.status === "running"
            ? "Working"
            : turn.status.replace("_", " ")}
        </span>
      </div>
      {hiddenCount > 0 ? (
        <Accordion.Root className="activity-accordion">
          <Accordion.Item value={turn.key}>
            <Accordion.Header>
              <Accordion.Trigger
                className="activity-trigger"
                aria-label={expandedLabel}
              >
                <span>{hiddenCount} earlier steps</span>
                <IconChevronDown size={15} stroke={1.7} aria-hidden="true" />
              </Accordion.Trigger>
            </Accordion.Header>
            <Accordion.Panel className="activity-panel">
              <ActivityRows items={hiddenItems} />
            </Accordion.Panel>
          </Accordion.Item>
        </Accordion.Root>
      ) : null}
      <ActivityRows items={visibleItems} />
    </section>
  );
}

import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";

function formatAgentCount(count: number) {
  return `${count} ${count === 1 ? "agent" : "agents"}`;
}

export function formatWorkingTooltip(
  summary: ActiveChannelTurnSummary,
): string {
  const leadName = summary.agentNames?.[0];

  if (!leadName) {
    return `${formatAgentCount(summary.agentCount)} working`;
  }

  const remainingAgentCount = summary.agentCount - 1;
  if (remainingAgentCount <= 0) {
    return `${leadName} working`;
  }

  return `${leadName} and ${formatAgentCount(remainingAgentCount)} working`;
}

export function ChannelWorkingBadge({
  channelName,
  isActive,
  summary,
  testId,
}: {
  channelName: string;
  isActive: boolean;
  summary: ActiveChannelTurnSummary;
  /** Override test id (defaults to channel-working-<channelName>). */
  testId?: string;
}) {
  const now = useNow(1000);
  const elapsed = formatElapsed(now - summary.anchorAt);
  const label =
    summary.agentCount > 1 ? `${elapsed} (${summary.agentCount})` : elapsed;
  const title = formatWorkingTooltip(summary);

  return (
    <span
      className={cn(
        "max-w-32 shrink-0 truncate rounded-full px-1.5 py-0.5 text-2xs font-medium leading-none tabular-nums motion-safe:animate-pulse group-data-[collapsible=icon]:hidden",
        "hidden sm:inline-flex",
        isActive
          ? "bg-sidebar-active-foreground/20 text-sidebar-active-foreground"
          : "bg-primary/10 text-primary",
      )}
      data-testid={testId ?? `channel-working-${channelName}`}
      title={title}
    >
      {label}
    </span>
  );
}

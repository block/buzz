import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { ChannelWorkingIndicator } from "@/features/sidebar/ui/SidebarSection";

export function SearchChannelActivityIndicator({
  channelName,
  summary,
  timestampLabel,
}: {
  channelName: string;
  summary?: ActiveChannelTurnSummary;
  timestampLabel: string | null;
}) {
  if (summary) {
    return (
      <ChannelWorkingIndicator
        channelName={channelName}
        className="inline-flex text-muted-foreground/60"
        isActive={false}
        summary={summary}
      />
    );
  }

  return timestampLabel ? (
    <span className="shrink-0 text-2xs text-muted-foreground/75">
      {timestampLabel}
    </span>
  ) : null;
}

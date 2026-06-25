import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";

type AgentWorkingBadgeProps = {
  anchorAt: number;
  channelId?: string;
  name: string;
  onNavigate?: (channelId: string) => void;
  variant: "panel" | "popover";
};

export function AgentWorkingBadge({
  anchorAt,
  channelId,
  name,
  onNavigate,
  variant,
}: AgentWorkingBadgeProps) {
  const now = useNow(1000);
  const label = `Working in #${name} · ${formatElapsed(now - anchorAt)}`;

  if (variant === "panel") {
    return (
      <Badge
        className={cn(
          "cursor-pointer motion-safe:animate-pulse normal-case tracking-normal hover:opacity-80",
          !onNavigate && "cursor-default",
        )}
        onClick={
          onNavigate && channelId ? () => onNavigate(channelId) : undefined
        }
        variant="default"
      >
        {label}
      </Badge>
    );
  }

  return (
    <span className="inline-flex items-center rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary motion-safe:animate-pulse">
      {label}
    </span>
  );
}

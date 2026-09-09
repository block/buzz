import { useContext } from "react";
import { CircleAlert } from "lucide-react";
import { TaskChannelBranches } from "@/features/channels/useChannelBackedTaskParents";
import { useTaskBranchStatus } from "@/features/channels/useTaskBranchStatus";
import type { ChannelBackedTask } from "@/features/channels/lib/channelBackedTask";
import { cn } from "@/shared/lib/cn";

function BranchGlyph({
  branch,
}: {
  branch: NonNullable<ChannelBackedTask["branch"]>;
}) {
  const { query, pr, Icon, label, color } = useTaskBranchStatus(branch);
  const description = `${label}: ${branch.name}${pr ? ` (#${pr.number})` : ""}${query.error ? " — PR status unavailable" : query.isFetching ? " — checking PR status" : ""}`;
  return (
    <span
      title={description}
      className="ml-auto flex shrink-0"
      data-testid="task-channel-status"
    >
      {query.error && !pr ? (
        <CircleAlert
          role="img"
          aria-label={description}
          className="h-3.5 w-3.5 text-muted-foreground"
        />
      ) : pr ? (
        <Icon
          role="img"
          aria-label={description}
          className={cn("h-3.5 w-3.5", color)}
        />
      ) : (
        <svg
          role="img"
          aria-label={description}
          className={cn("h-3.5 w-3.5", color)}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
        >
          <path d="M6 7v10" />
          <path d="M18 4v13" strokeDasharray="1 4" />
          <circle cx="6" cy="4" r="2.5" />
          <circle cx="6" cy="20" r="2.5" />
          <circle cx="18" cy="20" r="2.5" />
        </svg>
      )}
    </span>
  );
}

export function TaskChannelStatusGlyph({ channelId }: { channelId: string }) {
  const branch = useContext(TaskChannelBranches).get(channelId);
  return branch ? <BranchGlyph branch={branch} /> : null;
}

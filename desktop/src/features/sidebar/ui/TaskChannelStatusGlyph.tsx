import { useContext } from "react";
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
      <Icon
        role="img"
        aria-label={description}
        className={cn("h-3.5 w-3.5", color)}
      />
    </span>
  );
}

export function TaskChannelStatusGlyph({ channelId }: { channelId: string }) {
  const branch = useContext(TaskChannelBranches).get(channelId);
  return branch ? <BranchGlyph branch={branch} /> : null;
}

import { UserPlus } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import type { ChannelTask } from "@/features/tasks/lib/channelTasks";
import {
  type OwnerCandidate,
  type OwnerSuggestionReason,
  suggestTaskOwners,
} from "@/features/tasks/lib/ownerSuggestion";
import { useSetChannelTaskAssignee } from "@/features/tasks/lib/useTaskAssignee";
import { Button } from "@/shared/ui/button";

/**
 * REG-16: suggested-owner affordance for an UNASSIGNED channel task.
 *
 * Advisory only. Clicking a suggestion performs the ordinary assign write the
 * user could already perform by hand; the relay remains the sole authority and
 * a rejection surfaces as a visible error (never silence). The existing manual
 * assign path is not replaced.
 */

/** Reason codes are a closed enum; the UI owns the wording. */
function reasonText(reason: OwnerSuggestionReason): string {
  switch (reason.code) {
    case "mentioned-in-task":
      return "named in the task";
    case "channel-member":
      return "channel member";
    case "recent-participant":
      return "recently active here";
    case "agent-capability":
      return "active agent";
    case "task-author":
      return "filed this task";
    case "light-workload":
      return "no open tasks";
    default:
      // Unknown codes degrade to nothing rather than throwing.
      return "";
  }
}

export function SuggestedOwners({
  task,
  candidates,
  recentParticipantPubkeys,
  openTaskCountByPubkey,
}: {
  task: ChannelTask;
  candidates: readonly OwnerCandidate[];
  recentParticipantPubkeys?: readonly string[];
  openTaskCountByPubkey?: ReadonlyMap<string, number>;
}) {
  const setAssignee = useSetChannelTaskAssignee();

  const suggestions = React.useMemo(
    () =>
      suggestTaskOwners(task, candidates, {
        recentParticipantPubkeys,
        openTaskCountByPubkey,
      }),
    [task, candidates, recentParticipantPubkeys, openTaskCountByPubkey],
  );

  // FM1: render nothing at all rather than an empty panel. Also covers the
  // already-assigned case, which suggestTaskOwners returns [] for.
  if (suggestions.length === 0) {
    return null;
  }

  const onAssign = (pubkey: string) => {
    // FM2: re-check against the freshly rendered task, never stale props.
    if (task.assignee !== null) {
      toast.info("Task was already assigned");
      return;
    }
    setAssignee.mutate(
      { taskId: task.id, assignee: pubkey },
      {
        onError: (error: unknown) => {
          // FM5: a failed assign must produce a VISIBLE negative.
          toast.error(
            error instanceof Error ? error.message : "Could not assign task",
          );
        },
      },
    );
  };

  return (
    <div
      className="flex flex-wrap items-center gap-1.5 px-2 pb-1.5"
      data-testid="suggested-owners"
    >
      <span className="text-xs text-muted-foreground">Suggested:</span>
      {suggestions.map((suggestion) => {
        const why = suggestion.reasons
          .map(reasonText)
          .filter((text) => text !== "")
          .join(" · ");
        return (
          <Button
            className="h-6 gap-1 px-2 text-xs"
            data-testid="suggested-owner"
            data-pubkey={suggestion.pubkey}
            disabled={setAssignee.isPending}
            key={suggestion.pubkey}
            onClick={() => onAssign(suggestion.pubkey)}
            size="sm"
            title={why}
            type="button"
            variant="outline"
          >
            <UserPlus className="h-3 w-3" />
            {suggestion.label}
          </Button>
        );
      })}
    </div>
  );
}

import { CheckCircle2, Circle, Plus } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  isTaskDone,
  type ChannelTask,
} from "@/features/tasks/lib/channelTasks";
import { useChannelTasks, useCreateChannelTask, useSetChannelTaskStatus } from "@/features/tasks/lib/useChannelTasks";
import type { OwnerCandidate } from "@/features/tasks/lib/ownerSuggestion";
import { SuggestedOwners } from "@/features/tasks/ui/SuggestedOwners";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";

/**
 * REG-15: channel task list on desktop.
 *
 * "Channel task" naming per work/REG-15/reflecting.md D4 (Option A) — Projects'
 * kind:30617 issues are a different substrate; this view is the 0033 task
 * table over `/api/tasks`. v1 slice: list + create + complete/reopen.
 */
export function ChannelTaskList({ channelId }: { channelId: string | null }) {
  const tasks = useChannelTasks(channelId);
  const createTask = useCreateChannelTask(channelId);
  const setStatus = useSetChannelTaskStatus();
  const [draft, setDraft] = React.useState("");

  /**
   * REG-16 v1 candidate source: the people already visible in this task list.
   * Deliberately derived from data the client ALREADY has (task authors and
   * current assignees) rather than a new roster fetch — a suggestion is not a
   * grant, and the relay stays the sole authority on who may be assigned.
   * A richer channel-member/agent source is the next slice.
   */
  const ownerCandidates: OwnerCandidate[] = React.useMemo(() => {
    const seen = new Map<string, OwnerCandidate>();
    for (const task of tasks.data ?? []) {
      for (const pubkey of [task.createdBy, task.assignee]) {
        if (pubkey && !seen.has(pubkey)) {
          seen.set(pubkey, {
            kind: "identity",
            displayName: null,
            isAgent: false,
            isMember: true,
            pubkey,
          });
        }
      }
    }
    return [...seen.values()];
  }, [tasks.data]);

  const onToggle = (task: ChannelTask) => {
    setStatus.mutate(
      { taskId: task.id, status: isTaskDone(task) ? "open" : "done" },
      {
        onError: (error: unknown) => {
          toast.error(
            error instanceof Error ? error.message : "Could not update task",
          );
        },
      },
    );
  };

  const onCreate = () => {
    const title = draft.trim();
    if (title === "") {
      return;
    }
    createTask.mutate(title, {
      onSuccess: () => {
        setDraft("");
        toast.success("Task created");
      },
      onError: (error: unknown) => {
        toast.error(
          error instanceof Error ? error.message : "Could not create task",
        );
      },
    });
  };

  return (
    <div className="flex min-h-0 flex-col gap-3">
      <div className="flex items-center gap-2">
        <Input
          data-testid="channel-task-new-title"
          disabled={channelId === null || createTask.isPending}
          maxLength={200}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              onCreate();
            }
          }}
          placeholder="Add a channel task…"
          value={draft}
        />
        <Button
          data-testid="channel-task-create"
          disabled={channelId === null || draft.trim() === "" || createTask.isPending}
          onClick={onCreate}
          size="sm"
          type="button"
          variant="outline"
        >
          <Plus className="h-4 w-4" />
          Add
        </Button>
      </div>
      {tasks.isError ? (
        <p className="text-sm text-muted-foreground" data-testid="channel-task-error">
          {tasks.error instanceof Error
            ? tasks.error.message
            : "Could not load tasks"}
        </p>
      ) : null}
      {tasks.data ? (
        tasks.data.length === 0 ? (
          <p className="text-sm text-muted-foreground" data-testid="channel-task-empty">
            No channel tasks yet.
          </p>
        ) : (
          <ul className="flex flex-col gap-1" data-testid="channel-task-list">
            {tasks.data.map((task) => {
              const done = isTaskDone(task);
              return (
                <li key={task.id}>
                  <button
                    className={cn(
                      "flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left text-sm hover:bg-sidebar-accent",
                      done && "text-muted-foreground line-through",
                    )}
                    data-testid="channel-task-row"
                    data-task-id={task.id}
                    disabled={setStatus.isPending}
                    onClick={() => onToggle(task)}
                    type="button"
                  >
                    {done ? (
                      <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
                    ) : (
                      <Circle className="mt-0.5 h-4 w-4 shrink-0" />
                    )}
                    <span className="min-w-0 break-words">{task.title}</span>
                  </button>
                  {task.assignee === null ? (
                    <SuggestedOwners candidates={ownerCandidates} task={task} />
                  ) : null}
                </li>
              );
            })}
          </ul>
        )
      ) : null}
    </div>
  );
}

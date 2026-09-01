import {
  AlertCircle,
  CheckCircle2,
  Circle,
  Clock3,
  Play,
  RefreshCcw,
  RotateCcw,
  UserRound,
} from "lucide-react";
import * as React from "react";

import {
  DONE_STATUSES,
  type ChannelTask,
} from "@/features/tasks/lib/channelTasks";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";

type MyWorkViewProps = {
  tasks: ChannelTask[];
  currentPubkey?: string | null;
  channelNamesById?: ReadonlyMap<string, string>;
  isLoading: boolean;
  error: unknown;
  isStatusPending: boolean;
  onRetry: () => void;
  onSetStatus: (
    task: ChannelTask,
    status: "open" | "in_progress" | "done",
  ) => void;
};

type WorkGroup = {
  id: "needs-you" | "in-progress" | "queued" | "done";
  label: string;
  testId: string;
  tasks: ChannelTask[];
};

function normalizedPubkey(value: string | null | undefined) {
  return value?.trim().toLowerCase() ?? null;
}

export function groupMyWorkTasks(
  tasks: readonly ChannelTask[],
  currentPubkey?: string | null,
): WorkGroup[] {
  const me = normalizedPubkey(currentPubkey);
  const sorted = [...tasks].sort(
    (left, right) => right.updatedAt - left.updatedAt,
  );
  const needsYou: ChannelTask[] = [];
  const inProgress: ChannelTask[] = [];
  const queued: ChannelTask[] = [];
  const done: ChannelTask[] = [];

  for (const task of sorted) {
    if (DONE_STATUSES.has(task.status)) {
      done.push(task);
    } else if (me !== null && normalizedPubkey(task.assignee) === me) {
      needsYou.push(task);
    } else if (task.status === "in_progress") {
      inProgress.push(task);
    } else {
      queued.push(task);
    }
  }

  return [
    {
      id: "needs-you",
      label: "Needs you",
      testId: "my-work-needs-you",
      tasks: needsYou,
    },
    {
      id: "in-progress",
      label: "In progress",
      testId: "my-work-in-progress",
      tasks: inProgress,
    },
    { id: "queued", label: "Queued", testId: "my-work-queued", tasks: queued },
    { id: "done", label: "Done", testId: "my-work-done", tasks: done },
  ];
}

function statusLabel(status: string) {
  if (DONE_STATUSES.has(status)) return "Done";
  if (status === "in_progress") return "In progress";
  if (status === "open" || status === "todo") return "Open";
  return status.replaceAll("_", " ");
}

function formatTimestamp(seconds: number) {
  if (!Number.isFinite(seconds) || seconds <= 0) return "Time unavailable";
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(seconds * 1_000));
}

function channelLabel(
  task: ChannelTask,
  channelNamesById?: ReadonlyMap<string, string>,
) {
  if (!task.channelId) return "Community-wide";
  return `#${channelNamesById?.get(task.channelId) ?? task.channelId}`;
}

function identityLabel(pubkey: string | null) {
  return pubkey ? truncatePubkey(pubkey) : "Unassigned";
}

function WorkStatus({ task }: { task: ChannelTask }) {
  const done = DONE_STATUSES.has(task.status);
  const inProgress = task.status === "in_progress";
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2 py-1 text-xs font-medium",
        done && "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
        inProgress && "bg-blue-500/10 text-blue-700 dark:text-blue-300",
        !done && !inProgress && "bg-muted text-muted-foreground",
      )}
      data-testid="my-work-detail-status"
    >
      {done ? (
        <CheckCircle2 className="h-3.5 w-3.5" />
      ) : (
        <Circle className="h-3.5 w-3.5" />
      )}
      {statusLabel(task.status)}
    </span>
  );
}

function WorkRow({
  task,
  selected,
  channelNamesById,
  onSelect,
}: {
  task: ChannelTask;
  selected: boolean;
  channelNamesById?: ReadonlyMap<string, string>;
  onSelect: () => void;
}) {
  return (
    <button
      aria-current={selected ? "true" : undefined}
      className={cn(
        "group flex w-full gap-3 border-t border-border/45 px-4 py-3 text-left transition-colors hover:bg-muted/45 focus-visible:bg-muted/45 focus-visible:outline-hidden",
        selected && "bg-primary/8 shadow-[inset_3px_0_0_hsl(var(--primary))]",
      )}
      data-work-id={task.id}
      onClick={onSelect}
      type="button"
    >
      <span
        className={cn(
          "mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-xs font-semibold",
          DONE_STATUSES.has(task.status)
            ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
            : "bg-primary/10 text-primary",
        )}
      >
        {task.assignee ? identityLabel(task.assignee).slice(0, 2) : "—"}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-semibold text-foreground">
          {task.title || "Untitled task"}
        </span>
        <span className="mt-1 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
          <span className="truncate">
            {channelLabel(task, channelNamesById)}
          </span>
          <span aria-hidden="true">·</span>
          <span className="shrink-0">{statusLabel(task.status)}</span>
        </span>
        {task.body ? (
          <span className="mt-1.5 line-clamp-2 block text-xs leading-relaxed text-muted-foreground">
            {task.body}
          </span>
        ) : null}
      </span>
    </button>
  );
}

function EmptyState() {
  return (
    <div
      className="flex flex-1 flex-col items-center justify-center px-8 text-center"
      data-testid="my-work-empty"
    >
      <CheckCircle2 className="mb-3 h-9 w-9 text-muted-foreground/50" />
      <h2 className="text-base font-semibold text-foreground">
        No work items yet
      </h2>
      <p className="mt-1 max-w-sm text-sm text-muted-foreground">
        Tasks created from a channel or connected request source will appear
        here.
      </p>
    </div>
  );
}

export function MyWorkView({
  tasks,
  currentPubkey,
  channelNamesById,
  isLoading,
  error,
  isStatusPending,
  onRetry,
  onSetStatus,
}: MyWorkViewProps) {
  const groups = React.useMemo(
    () => groupMyWorkTasks(tasks, currentPubkey),
    [currentPubkey, tasks],
  );
  const orderedTasks = React.useMemo(
    () => groups.flatMap((group) => group.tasks),
    [groups],
  );
  const [selectedTaskId, setSelectedTaskId] = React.useState<string | null>(
    null,
  );
  const selectedTask =
    orderedTasks.find((task) => task.id === selectedTaskId) ??
    orderedTasks[0] ??
    null;

  React.useEffect(() => {
    if (
      selectedTaskId !== null &&
      !tasks.some((task) => task.id === selectedTaskId)
    ) {
      setSelectedTaskId(null);
    }
  }, [selectedTaskId, tasks]);

  if (isLoading) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading work…
      </div>
    );
  }

  if (error) {
    return (
      <div
        className="flex flex-1 flex-col items-center justify-center px-8 text-center"
        data-testid="my-work-error"
      >
        <AlertCircle className="mb-3 h-9 w-9 text-destructive" />
        <h2 className="text-base font-semibold text-foreground">
          Could not load My Work
        </h2>
        <p className="mt-1 max-w-md text-sm text-muted-foreground">
          {error instanceof Error
            ? error.message
            : "The task service is unavailable."}
        </p>
        <Button
          className="mt-4"
          data-testid="my-work-retry"
          onClick={onRetry}
          size="sm"
          type="button"
          variant="outline"
        >
          <RefreshCcw className="h-4 w-4" />
          Try again
        </Button>
      </div>
    );
  }

  if (orderedTasks.length === 0) return <EmptyState />;

  return (
    <div className="grid min-h-0 flex-1 grid-cols-[minmax(17rem,24rem)_minmax(0,1fr)] max-[820px]:grid-cols-1">
      <aside className="min-h-0 overflow-y-auto border-r border-border/60 bg-background max-[820px]:max-h-[45%] max-[820px]:border-b max-[820px]:border-r-0">
        {groups.map((group) => (
          <section data-testid={group.testId} key={group.id}>
            <div className="flex items-center px-4 pb-2 pt-4 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
              {group.label}
              <span className="ml-auto font-normal">{group.tasks.length}</span>
            </div>
            {group.tasks.length === 0 ? (
              <p className="border-t border-border/45 px-4 py-3 text-xs text-muted-foreground/70">
                Nothing here
              </p>
            ) : (
              group.tasks.map((task) => (
                <WorkRow
                  channelNamesById={channelNamesById}
                  key={task.id}
                  onSelect={() => setSelectedTaskId(task.id)}
                  selected={selectedTask?.id === task.id}
                  task={task}
                />
              ))
            )}
          </section>
        ))}
      </aside>

      {selectedTask ? (
        <article className="min-h-0 overflow-y-auto bg-background/70">
          <div className="mx-auto max-w-3xl px-6 py-6 sm:px-8">
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <span>{channelLabel(selectedTask, channelNamesById)}</span>
              {selectedTask.source ? (
                <>
                  <span aria-hidden="true">›</span>
                  <span data-testid="my-work-detail-source">
                    {selectedTask.source}
                  </span>
                </>
              ) : null}
            </div>
            <h2
              className="mt-2 text-pretty text-2xl font-semibold tracking-tight text-foreground"
              data-testid="my-work-detail-title"
            >
              {selectedTask.title || "Untitled task"}
            </h2>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <WorkStatus task={selectedTask} />
              <span className="inline-flex items-center gap-1.5 rounded-md border bg-background px-2 py-1 text-xs text-muted-foreground">
                <UserRound className="h-3.5 w-3.5" />
                {identityLabel(selectedTask.assignee)}
              </span>
              {selectedTask.priority > 0 ? (
                <span className="rounded-md border bg-background px-2 py-1 text-xs text-muted-foreground">
                  Priority {selectedTask.priority}
                </span>
              ) : null}
            </div>

            {selectedTask.body ? (
              <section className="mt-5 rounded-xl border bg-background p-4 shadow-xs">
                <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                  Request
                </h3>
                <p
                  className="mt-2 whitespace-pre-wrap text-sm leading-relaxed text-foreground"
                  data-testid="my-work-request-body"
                >
                  {selectedTask.body}
                </p>
                {selectedTask.sourceRef ? (
                  <p className="mt-3 break-all font-mono text-xs text-muted-foreground">
                    Source reference: {selectedTask.sourceRef}
                  </p>
                ) : null}
              </section>
            ) : null}

            <section className="mt-5 rounded-xl border bg-background p-4 shadow-xs">
              <div className="flex items-start gap-3">
                <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
                  <Clock3 className="h-4 w-4" />
                </span>
                <div className="min-w-0 flex-1">
                  <h3 className="text-sm font-semibold text-foreground">
                    Current state
                  </h3>
                  <p className="mt-1 text-sm text-muted-foreground">
                    {statusLabel(selectedTask.status)} · updated{" "}
                    {formatTimestamp(selectedTask.updatedAt)}
                  </p>
                  <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                    Detailed agent execution remains in the activity transcript.
                    My Work only shows task state the relay can verify.
                  </p>
                </div>
              </div>
            </section>

            <div className="mt-5 flex flex-wrap gap-2">
              {!DONE_STATUSES.has(selectedTask.status) &&
              selectedTask.status !== "in_progress" ? (
                <Button
                  data-testid="my-work-start-task"
                  disabled={isStatusPending}
                  onClick={() => onSetStatus(selectedTask, "in_progress")}
                  size="sm"
                  type="button"
                >
                  <Play className="h-4 w-4" />
                  Start work
                </Button>
              ) : null}
              {!DONE_STATUSES.has(selectedTask.status) ? (
                <Button
                  data-testid="my-work-complete-task"
                  disabled={isStatusPending}
                  onClick={() => onSetStatus(selectedTask, "done")}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <CheckCircle2 className="h-4 w-4" />
                  Mark done
                </Button>
              ) : (
                <Button
                  data-testid="my-work-reopen-task"
                  disabled={isStatusPending}
                  onClick={() => onSetStatus(selectedTask, "open")}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <RotateCcw className="h-4 w-4" />
                  Reopen
                </Button>
              )}
            </div>

            <dl className="mt-7 grid grid-cols-1 gap-4 border-t pt-5 text-xs sm:grid-cols-2">
              <div>
                <dt className="text-muted-foreground">Created by</dt>
                <dd className="mt-1 font-mono text-foreground">
                  {identityLabel(selectedTask.createdBy)}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Created</dt>
                <dd className="mt-1 text-foreground">
                  {formatTimestamp(selectedTask.createdAt)}
                </dd>
              </div>
              {selectedTask.dueAt ? (
                <div>
                  <dt className="text-muted-foreground">Due</dt>
                  <dd className="mt-1 text-foreground">
                    {formatTimestamp(selectedTask.dueAt)}
                  </dd>
                </div>
              ) : null}
              <div>
                <dt className="text-muted-foreground">Task ID</dt>
                <dd className="mt-1 break-all font-mono text-foreground">
                  {selectedTask.id}
                </dd>
              </div>
            </dl>
          </div>
        </article>
      ) : null}
    </div>
  );
}

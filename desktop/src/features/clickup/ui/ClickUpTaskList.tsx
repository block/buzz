import { CalendarDays, ChevronRight, RefreshCw, Search } from "lucide-react";

import {
  CLICKUP_URGENCY_GROUPS,
  clickUpTaskFilterOptions,
  filterClickUpTasks,
  groupClickUpTasks,
  taskLocationLabel,
  type ClickUpTaskFilters,
} from "@/features/clickup/lib/tasks";
import type { ClickUpApiError, ClickUpTask } from "@/features/clickup/types";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";

type ClickUpTaskListProps = {
  error: ClickUpApiError | null;
  fetchedAtMs: number | null;
  filters: ClickUpTaskFilters;
  isFetching: boolean;
  isLoading: boolean;
  onFiltersChange: (filters: ClickUpTaskFilters) => void;
  onRefresh: () => void;
  onSelectTask: (taskId: string) => void;
  retryReady: boolean;
  selectedTaskId: string | null;
  tasks: ClickUpTask[];
  truncated: boolean;
};

const dueFormatter = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
});

const refreshedFormatter = new Intl.DateTimeFormat(undefined, {
  hour: "numeric",
  minute: "2-digit",
});

function formatDueDate(value: string | null) {
  if (!value) return "No due date";
  const timestamp = Number(value);
  return Number.isFinite(timestamp)
    ? dueFormatter.format(new Date(timestamp))
    : "No due date";
}

function errorCopy(error: ClickUpApiError) {
  if (error.code === "rate_limited")
    return "ClickUp is temporarily limiting requests. Your last loaded tasks are still shown.";
  if (error.code === "forbidden")
    return "This Workspace is not available to the connected account. Choose another Workspace.";
  if (error.code === "network")
    return "Buzz could not reach ClickUp. Your last loaded tasks are still shown when available.";
  return error.message;
}

function TaskListSkeleton() {
  return (
    <div className="space-y-3" data-testid="clickup-task-skeleton">
      {["first", "second", "third", "fourth"].map((key) => (
        <Card className="space-y-3 p-4" key={key}>
          <div className="flex items-center justify-between gap-3">
            <Skeleton className="h-4 w-48 max-w-2/3" />
            <Skeleton className="h-5 w-16 rounded-full" />
          </div>
          <Skeleton className="h-3 w-2/3" />
        </Card>
      ))}
    </div>
  );
}

export function ClickUpTaskList({
  error,
  fetchedAtMs,
  filters,
  isFetching,
  isLoading,
  onFiltersChange,
  onRefresh,
  onSelectTask,
  retryReady,
  selectedTaskId,
  tasks,
  truncated,
}: ClickUpTaskListProps) {
  const options = clickUpTaskFilterOptions(tasks);
  const filteredTasks = filterClickUpTasks(tasks, filters);
  const groupedTasks = groupClickUpTasks(filteredTasks);
  const hasActiveFilters =
    filters.search.trim().length > 0 ||
    filters.status !== "all" ||
    filters.priority !== "all" ||
    filters.location !== "all" ||
    filters.dueWindow !== "all";

  const updateFilter = <Key extends keyof ClickUpTaskFilters>(
    key: Key,
    value: ClickUpTaskFilters[Key],
  ) => onFiltersChange({ ...filters, [key]: value });

  return (
    <section className="min-w-0 space-y-4 lg:col-span-2">
      {truncated ? (
        <div className="rounded-xl border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-950 dark:text-amber-100">
          Results are incomplete because Buzz loaded the first 2,000 assigned
          tasks. Open ClickUp to search the full Workspace.
        </div>
      ) : null}
      <div className="flex flex-col gap-3 rounded-xl border border-border/60 bg-card/50 p-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
          <div className="relative min-w-0 flex-1">
            <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              aria-label="Search loaded ClickUp tasks"
              className="pl-9"
              data-testid="clickup-search"
              onChange={(event) => updateFilter("search", event.target.value)}
              placeholder="Search loaded tasks"
              value={filters.search}
            />
          </div>
          <Button
            data-testid="refresh-clickup-tasks"
            disabled={isFetching || !retryReady}
            onClick={onRefresh}
            size="sm"
            variant="outline"
          >
            <RefreshCw className={cn(isFetching && "animate-spin")} />
            {isFetching ? "Refreshing…" : "Refresh"}
          </Button>
        </div>
        <div className="grid gap-2 sm:grid-cols-2 xl:grid-cols-4">
          <select
            aria-label="Filter ClickUp tasks by status"
            className="h-8 rounded-lg border border-input/40 bg-background px-2 text-xs outline-hidden focus:ring-1 focus:ring-ring"
            onChange={(event) => updateFilter("status", event.target.value)}
            value={filters.status}
          >
            <option value="all">All statuses</option>
            {options.statuses.map((status) => (
              <option key={status} value={status}>
                {status}
              </option>
            ))}
          </select>
          <select
            aria-label="Filter ClickUp tasks by due window"
            className="h-8 rounded-lg border border-input/40 bg-background px-2 text-xs outline-hidden focus:ring-1 focus:ring-ring"
            onChange={(event) =>
              updateFilter(
                "dueWindow",
                event.target.value as ClickUpTaskFilters["dueWindow"],
              )
            }
            value={filters.dueWindow}
          >
            <option value="all">All due dates</option>
            {CLICKUP_URGENCY_GROUPS.map((group) => (
              <option key={group.key} value={group.key}>
                {group.label}
              </option>
            ))}
          </select>
          <select
            aria-label="Filter ClickUp tasks by priority"
            className="h-8 rounded-lg border border-input/40 bg-background px-2 text-xs outline-hidden focus:ring-1 focus:ring-ring"
            onChange={(event) => updateFilter("priority", event.target.value)}
            value={filters.priority}
          >
            <option value="all">All priorities</option>
            {options.priorities.map((priority) => (
              <option key={priority} value={priority}>
                {priority === "none" ? "No priority" : priority}
              </option>
            ))}
          </select>
          <select
            aria-label="Filter ClickUp tasks by location"
            className="h-8 rounded-lg border border-input/40 bg-background px-2 text-xs outline-hidden focus:ring-1 focus:ring-ring"
            onChange={(event) => updateFilter("location", event.target.value)}
            value={filters.location}
          >
            <option value="all">All locations</option>
            {options.locations.map((location) => (
              <option key={location.id} value={location.id}>
                {location.label}
              </option>
            ))}
          </select>
        </div>
        <div className="flex min-h-5 items-center justify-between gap-3 text-xs text-muted-foreground">
          <span aria-live="polite">
            {isLoading
              ? "Loading your ClickUp tasks…"
              : `${filteredTasks.length} open assigned ${filteredTasks.length === 1 ? "task" : "tasks"}`}
          </span>
          {fetchedAtMs ? (
            <span>Last refreshed {refreshedFormatter.format(fetchedAtMs)}</span>
          ) : null}
        </div>
      </div>

      {error ? (
        <div
          className="flex flex-wrap items-center justify-between gap-3 rounded-xl bg-amber-500/10 px-3 py-2 text-xs leading-5 text-amber-700 dark:text-amber-300"
          data-testid="clickup-task-error"
          role="alert"
        >
          <span>{errorCopy(error)}</span>
          <Button
            disabled={isFetching || !retryReady}
            onClick={onRefresh}
            size="sm"
            variant="outline"
          >
            {isFetching ? "Trying again…" : "Try again"}
          </Button>
        </div>
      ) : null}

      <div aria-busy={isLoading} aria-live="polite">
        {isLoading ? <TaskListSkeleton /> : null}
      </div>

      {!isLoading &&
      !(error && tasks.length === 0) &&
      filteredTasks.length === 0 ? (
        <Card className="flex min-h-48 flex-col items-center justify-center gap-3 p-6 text-center">
          <CalendarDays className="h-6 w-6 text-muted-foreground" />
          <div>
            <p className="text-sm font-medium">
              {hasActiveFilters
                ? "No tasks match these filters."
                : "You have no open assigned tasks in this Workspace."}
            </p>
            {hasActiveFilters ? (
              <Button
                className="mt-3"
                onClick={() =>
                  onFiltersChange({
                    search: "",
                    status: "all",
                    priority: "all",
                    location: "all",
                    dueWindow: "all",
                  })
                }
                size="sm"
                variant="outline"
              >
                Clear filters
              </Button>
            ) : null}
          </div>
        </Card>
      ) : null}

      {!isLoading
        ? CLICKUP_URGENCY_GROUPS.map((group) => {
            const groupTasks = groupedTasks[group.key];
            if (groupTasks.length === 0) return null;
            return (
              <div className="space-y-2" key={group.key}>
                <div className="flex items-center gap-2 px-1">
                  <h2 className="text-sm font-semibold">{group.label}</h2>
                  <span className="text-xs text-muted-foreground">
                    {groupTasks.length}
                  </span>
                </div>
                <div className="space-y-2">
                  {groupTasks.map((task) => (
                    <button
                      aria-pressed={selectedTaskId === task.id}
                      className={cn(
                        "flex w-full items-center gap-3 rounded-xl border border-border/60 bg-card/60 p-3 text-left transition-colors hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
                        selectedTaskId === task.id &&
                          "border-primary/40 bg-primary/5",
                      )}
                      data-testid={`clickup-task-${task.id}`}
                      id={`clickup-task-row-${task.id}`}
                      key={task.id}
                      onClick={() => onSelectTask(task.id)}
                      type="button"
                    >
                      <div className="min-w-0 flex-1 space-y-1.5">
                        <div className="flex min-w-0 items-center gap-2">
                          <p className="truncate text-sm font-medium">
                            {task.name}
                          </p>
                          <Badge className="shrink-0" variant="secondary">
                            {task.status.status}
                          </Badge>
                        </div>
                        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                          <span>{formatDueDate(task.dueDateMs)}</span>
                          <span>
                            {task.priority?.priority ?? "No priority"}
                          </span>
                          {taskLocationLabel(task) ? (
                            <span className="truncate">
                              {taskLocationLabel(task)}
                            </span>
                          ) : null}
                        </div>
                      </div>
                      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
                    </button>
                  ))}
                </div>
              </div>
            );
          })
        : null}
    </section>
  );
}

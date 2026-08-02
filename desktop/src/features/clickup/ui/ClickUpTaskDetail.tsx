import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ArrowUpRight, MessageSquareText, X } from "lucide-react";

import {
  useClickUpTask,
  useClickUpTaskComments,
} from "@/features/clickup/hooks";
import { taskLocationLabel } from "@/features/clickup/lib/tasks";
import type { ClickUpCustomField } from "@/features/clickup/types";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Skeleton } from "@/shared/ui/skeleton";

type ClickUpTaskDetailProps = {
  onClose: () => void;
  pubkey: string;
  taskId: string;
  workspaceId: string;
};

const dateTimeFormatter = new Intl.DateTimeFormat(undefined, {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

function formatTimestamp(value: string | null) {
  if (!value) return "Not set";
  const timestamp = Number(value);
  return Number.isFinite(timestamp)
    ? dateTimeFormatter.format(new Date(timestamp))
    : "Not set";
}

function formatCustomFieldValue(field: ClickUpCustomField) {
  if (field.value === null || field.value === undefined || field.value === "")
    return null;
  if (["string", "number", "boolean"].includes(typeof field.value))
    return String(field.value);
  if (Array.isArray(field.value))
    return field.value.map((item) => String(item)).join(", ");
  return null;
}

function isSafeClickUpUrl(value: string) {
  try {
    const url = new URL(value);
    return (
      url.protocol === "https:" &&
      (url.hostname === "clickup.com" || url.hostname.endsWith(".clickup.com"))
    );
  } catch {
    return false;
  }
}

function DetailSkeleton() {
  return (
    <div className="space-y-5">
      <Skeleton className="h-6 w-3/4" />
      <Skeleton className="h-4 w-1/2" />
      <Skeleton className="h-20 w-full" />
      <Skeleton className="h-24 w-full" />
    </div>
  );
}

export function ClickUpTaskDetail({
  onClose,
  pubkey,
  taskId,
  workspaceId,
}: ClickUpTaskDetailProps) {
  const taskQuery = useClickUpTask(pubkey, workspaceId, taskId);
  const commentsQuery = useClickUpTaskComments(pubkey, workspaceId, taskId);
  const task = taskQuery.data;
  const headingRef = React.useRef<HTMLHeadingElement>(null);
  const headingId = `clickup-task-detail-heading-${taskId}`;

  React.useEffect(() => {
    if (task) headingRef.current?.focus();
  }, [task]);

  const close = React.useCallback(() => {
    const trigger = document.getElementById(`clickup-task-row-${taskId}`);
    onClose();
    window.requestAnimationFrame(() => trigger?.focus());
  }, [onClose, taskId]);
  const visibleFields =
    task?.customFields
      .map((field) => ({ field, value: formatCustomFieldValue(field) }))
      .filter((entry): entry is { field: ClickUpCustomField; value: string } =>
        Boolean(entry.value),
      ) ?? [];

  return (
    <aside
      aria-labelledby={headingId}
      className="min-w-0 lg:sticky lg:top-4 lg:self-start"
    >
      <Card className="max-h-[calc(100vh-8rem)] overflow-y-auto p-4">
        <div className="mb-4 flex items-center justify-between gap-3">
          <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Task details
          </p>
          <Button
            aria-label="Close task details"
            onClick={close}
            size="icon-xs"
            variant="ghost"
          >
            <X />
          </Button>
        </div>

        {taskQuery.isLoading ? <DetailSkeleton /> : null}
        {taskQuery.isError ? (
          <div className="space-y-3 text-sm">
            <p className="text-destructive">Could not load this task.</p>
            <Button
              onClick={() => void taskQuery.refetch()}
              size="sm"
              variant="outline"
            >
              Try again
            </Button>
          </div>
        ) : null}

        {task ? (
          <div className="space-y-6" data-testid="clickup-task-detail">
            <div className="space-y-2">
              <h2
                className="text-lg font-semibold leading-snug outline-hidden"
                id={headingId}
                ref={headingRef}
                tabIndex={-1}
              >
                {task.name}
              </h2>
              <p className="text-xs text-muted-foreground">
                {taskLocationLabel(task) || "ClickUp task"}
              </p>
              <div className="flex flex-wrap gap-2 text-xs">
                <span className="rounded-full bg-muted px-2 py-1">
                  {task.status.status}
                </span>
                <span className="rounded-full bg-muted px-2 py-1">
                  {task.priority?.priority ?? "No priority"}
                </span>
              </div>
            </div>

            <dl className="grid grid-cols-2 gap-3 text-xs">
              <div>
                <dt className="text-muted-foreground">Due</dt>
                <dd className="mt-1 font-medium">
                  {formatTimestamp(task.dueDateMs)}
                </dd>
              </div>
              <div>
                <dt className="text-muted-foreground">Updated</dt>
                <dd className="mt-1 font-medium">
                  {formatTimestamp(task.dateUpdatedMs)}
                </dd>
              </div>
              <div className="col-span-2">
                <dt className="text-muted-foreground">Assignees</dt>
                <dd className="mt-1 font-medium">
                  {task.assignees
                    .map((assignee) => assignee.username)
                    .join(", ") || "None"}
                </dd>
              </div>
            </dl>

            <section className="space-y-2">
              <h3 className="text-sm font-semibold">Description</h3>
              <p className="whitespace-pre-wrap text-sm leading-6 text-muted-foreground">
                {task.description || task.textContent || "No description."}
              </p>
            </section>

            {task.subtasks.length > 0 ? (
              <section className="space-y-2">
                <h3 className="text-sm font-semibold">
                  Subtasks ({task.subtasks.length})
                </h3>
                <div className="space-y-1.5">
                  {task.subtasks.map((subtask) => (
                    <div
                      className="rounded-lg bg-muted/50 px-3 py-2 text-xs"
                      key={subtask.id}
                    >
                      {subtask.name}
                    </div>
                  ))}
                </div>
              </section>
            ) : null}

            {visibleFields.length > 0 ? (
              <section className="space-y-2">
                <h3 className="text-sm font-semibold">Custom fields</h3>
                <dl className="space-y-2 text-xs">
                  {visibleFields.map(({ field, value }) => (
                    <div
                      className="rounded-lg bg-muted/40 px-3 py-2"
                      key={field.id}
                    >
                      <dt className="text-muted-foreground">{field.name}</dt>
                      <dd className="mt-1 font-medium">{value}</dd>
                    </div>
                  ))}
                </dl>
              </section>
            ) : null}

            {task.dependencies.length > 0 ? (
              <section className="space-y-2">
                <h3 className="text-sm font-semibold">
                  Dependencies ({task.dependencies.length})
                </h3>
                <div className="space-y-1 text-xs text-muted-foreground">
                  {task.dependencies.map((dependency) => (
                    <p
                      key={[
                        dependency.taskId,
                        dependency.dependsOn,
                        dependency.dependencyOf,
                      ]
                        .filter(Boolean)
                        .join(":")}
                    >
                      {dependency.dependsOn ??
                        dependency.dependencyOf ??
                        dependency.taskId ??
                        "Linked task"}
                    </p>
                  ))}
                </div>
              </section>
            ) : null}

            <section className="space-y-2">
              <div className="flex items-center gap-2">
                <MessageSquareText className="h-4 w-4" />
                <h3 className="text-sm font-semibold">Recent comments</h3>
              </div>
              {commentsQuery.isLoading ? (
                <Skeleton className="h-16 w-full" />
              ) : null}
              {commentsQuery.isError ? (
                <div
                  className="flex flex-wrap items-center justify-between gap-2 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive"
                  role="alert"
                >
                  <span>Could not load recent comments.</span>
                  <Button
                    disabled={commentsQuery.isFetching}
                    onClick={() => void commentsQuery.refetch()}
                    size="sm"
                    variant="outline"
                  >
                    Try again
                  </Button>
                </div>
              ) : null}
              {commentsQuery.data?.comments.length === 0 ? (
                <p className="text-xs text-muted-foreground">
                  No recent comments.
                </p>
              ) : null}
              <div className="space-y-2">
                {commentsQuery.data?.comments.slice(0, 5).map((comment) => (
                  <div
                    className="rounded-lg bg-muted/40 px-3 py-2"
                    key={comment.id}
                  >
                    <div className="flex items-center justify-between gap-2 text-xs">
                      <span className="font-medium">
                        {comment.user?.username ?? "ClickUp user"}
                      </span>
                      <span className="text-muted-foreground">
                        {formatTimestamp(comment.dateMs)}
                      </span>
                    </div>
                    <p className="mt-1 whitespace-pre-wrap text-xs leading-5 text-muted-foreground">
                      {comment.text || "Comment content unavailable."}
                    </p>
                  </div>
                ))}
              </div>
            </section>

            <Button
              className="w-full"
              disabled={!isSafeClickUpUrl(task.url)}
              onClick={() => void openUrl(task.url)}
              variant="outline"
            >
              Open in ClickUp
              <ArrowUpRight />
            </Button>
          </div>
        ) : null}
      </Card>
    </aside>
  );
}

import { openUrl } from "@tauri-apps/plugin-opener";
import type * as React from "react";
import { toast } from "sonner";
import { ListTodo } from "lucide-react";
import { useCanvasQuery } from "@/features/channels/hooks";
import {
  githubRepositoryUrl,
  parseChannelBackedTask,
  type ChannelBackedTask,
} from "@/features/channels/lib/channelBackedTask";
import { useTaskBranchStatus } from "@/features/channels/useTaskBranchStatus";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { AssignTask } from "./AssignTask";
import { TaskReviewModal } from "./TaskReviewModal";

function BranchStatus({
  branch,
}: {
  branch: NonNullable<ChannelBackedTask["branch"]>;
}) {
  const repository = githubRepositoryUrl(branch.repository);
  const { query, pr, Icon, label: status, color } = useTaskBranchStatus(branch);
  const href =
    pr?.url ??
    (repository
      ? `${repository}/tree/${encodeURIComponent(branch.name)}`
      : null);
  const label = branch.name;
  return (
    <div className="min-w-0 text-xs">
      <div className="flex items-center gap-2">
        <Icon aria-label={status} className={cn("h-4 w-4 shrink-0", color)} />
        {href ? (
          <a
            href={
              repository
                ? `${repository}/tree/${encodeURIComponent(branch.name)}`
                : href
            }
            target="_blank"
            rel="noreferrer"
            className="min-w-0 truncate text-muted-foreground hover:text-foreground hover:underline"
            title={`${status}: ${label}`}
            onClick={(event) => {
              event.preventDefault();
              void openUrl(
                repository
                  ? `${repository}/tree/${encodeURIComponent(branch.name)}`
                  : href,
              ).catch(() => toast.error("Could not open link in browser"));
            }}
          >
            {label}
          </a>
        ) : (
          <span>{label}</span>
        )}
        {pr && (
          <a
            href={pr.url}
            target="_blank"
            rel="noreferrer"
            className="shrink-0 text-muted-foreground hover:text-foreground hover:underline"
            title={pr.title}
            onClick={(event) => {
              event.preventDefault();
              void openUrl(pr.url).catch(() =>
                toast.error("Could not open link in browser"),
              );
            }}
          >
            #{pr.number}
          </a>
        )}
      </div>
      {query.isFetching && <span className="sr-only">Checking for PR…</span>}
      {query.error && (
        <p role="status" className="truncate" title={query.error.message}>
          Could not check PR
        </p>
      )}
    </div>
  );
}

export function TaskChannelWorkspace({
  channelId,
  children,
}: {
  channelId: string | null;
  children: React.ReactNode;
}) {
  const canvas = useCanvasQuery(channelId, channelId !== null);
  const task = parseChannelBackedTask(canvas.data?.content);
  if (!task) return children;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        className={cn(
          channelChrome.contentPadding,
          "shrink-0 border-b px-5 pb-2",
        )}
      >
        <section aria-label="Task" data-testid="task-overview">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <h2 className="flex min-w-0 items-center gap-2 text-sm font-medium">
              {!task.branch && (
                <ListTodo
                  aria-hidden="true"
                  data-testid="branchless-task-glyph"
                  className="h-4 w-4 shrink-0 text-muted-foreground"
                />
              )}
              <span className="truncate" title={task.task.title}>
                {task.task.title}
              </span>
            </h2>
            {task.branch && <BranchStatus branch={task.branch} />}
            {task.branch && <TaskReviewModal branch={task.branch} />}
            {channelId && canvas.data?.content && (
              <AssignTask
                key={channelId}
                channelId={channelId}
                content={canvas.data.content}
              />
            )}
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            {task.task.description}
          </p>
        </section>
      </div>
      <div
        className="flex min-h-0 flex-1 flex-col"
        style={
          { "--buzz-channel-content-top-padding": "0px" } as React.CSSProperties
        }
      >
        {children}
      </div>
    </div>
  );
}

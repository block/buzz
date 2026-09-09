import { openUrl } from "@tauri-apps/plugin-opener";
import type * as React from "react";
import { toast } from "sonner";
import { useCanvasQuery } from "@/features/channels/hooks";
import {
  githubRepositoryUrl,
  parseChannelBackedTask,
  type ChannelBackedTask,
} from "@/features/channels/lib/channelBackedTask";
import { useTaskBranchStatus } from "@/features/channels/useTaskBranchStatus";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Markdown } from "@/shared/ui/markdown";

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
  const label = pr ? `#${pr.number} ${pr.title}` : branch.name;
  return (
    <div className="mt-1 space-y-1 text-xs">
      <div className="flex items-center gap-2">
        <Icon aria-label={status} className={cn("h-4 w-4 shrink-0", color)} />
        {href ? (
          <a
            href={href}
            target="_blank"
            rel="noreferrer"
            className="min-w-0 truncate text-muted-foreground hover:text-foreground hover:underline"
            title={`${status}: ${label}`}
            onClick={(event) => {
              event.preventDefault();
              void openUrl(href).catch(() =>
                toast.error("Could not open link in browser"),
              );
            }}
          >
            {label}
          </a>
        ) : (
          <span>{label}</span>
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
          "shrink-0 border-b px-5 pb-3",
        )}
      >
        <details open data-testid="task-overview">
          <summary className="cursor-pointer text-sm font-medium">
            {task.task.title}
          </summary>
          <div
            className="mt-2 max-h-48 space-y-2 overflow-y-auto text-sm"
            data-testid="task-overview-details"
          >
            <p className="text-muted-foreground">{task.task.description}</p>
            {task.branch && <BranchStatus branch={task.branch} />}
            <div className="flex flex-wrap gap-x-6 gap-y-2 text-xs">
              <div>
                Origin <Markdown content={task.originatingThread} />
              </div>
              <div>
                Parent <Markdown content={task.parentChannel} />
              </div>
            </div>
          </div>
        </details>
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

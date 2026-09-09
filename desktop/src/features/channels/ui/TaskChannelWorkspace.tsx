import { openUrl } from "@tauri-apps/plugin-opener";
import * as React from "react";
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
  const [view, setView] = React.useState<"overview" | "conversation">(
    "overview",
  );
  if (!task) return children;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <nav
        aria-label="Task views"
        className={cn(
          channelChrome.contentPadding,
          "flex gap-1 border-b px-5 pb-2",
        )}
        data-testid="task-view-tabs"
      >
        {(["overview", "conversation"] as const).map((item) => (
          <button
            key={item}
            type="button"
            aria-current={view === item ? "page" : undefined}
            data-testid={`task-view-${item}`}
            className={cn(
              "rounded-full px-3 py-1.5 text-xs capitalize hover:bg-muted",
              view === item && "bg-muted",
            )}
            onClick={() => setView(item)}
          >
            {item}
          </button>
        ))}
      </nav>
      {view === "conversation" ? (
        children
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div
            className="mx-auto max-w-3xl space-y-6 px-6 py-8"
            data-testid="task-overview"
          >
            <h2 className="text-xl font-semibold">{task.task.title}</h2>
            <p className="text-sm text-muted-foreground">
              {task.task.description}
            </p>
            <div className="space-y-2 text-sm">
              <div>
                Origin <Markdown content={task.originatingThread} />
              </div>
              <div>
                Parent <Markdown content={task.parentChannel} />
              </div>
            </div>
            {task.branch && <BranchStatus branch={task.branch} />}
          </div>
        </div>
      )}
    </div>
  );
}

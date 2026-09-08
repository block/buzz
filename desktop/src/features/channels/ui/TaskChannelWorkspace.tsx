import { useQuery } from "@tanstack/react-query";
import { GitBranch, GitPullRequest, MessagesSquare } from "lucide-react";
import * as React from "react";

import { useCanvasQuery } from "@/features/channels/hooks";
import {
  githubRepositoryUrl,
  parseChannelBackedTask,
  type ChannelBackedTask,
} from "@/features/channels/lib/channelBackedTask";
import { getProjectRepoDiff } from "@/shared/api/projectGit";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Button } from "@/shared/ui/button";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { cn } from "@/shared/lib/cn";

type TaskView = "overview" | "changes" | "review" | "conversation";

const VIEWS: readonly TaskView[] = [
  "overview",
  "changes",
  "review",
  "conversation",
];

function TaskLink({ href, children }: { href: string; children: string }) {
  return (
    <a className="text-primary hover:underline" href={href}>
      {children}
    </a>
  );
}

function TaskOverview({ task }: { task: ChannelBackedTask }) {
  return (
    <div
      className="mx-auto w-full max-w-3xl space-y-6 px-6 py-8"
      data-testid="task-overview"
    >
      <div className="space-y-2">
        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Task
        </p>
        <h2 className="text-xl font-semibold tracking-tight">
          {task.task.title}
        </h2>
        <p className="text-sm leading-6 text-muted-foreground">
          {task.task.description}
        </p>
      </div>
      <dl className="grid gap-4 rounded-xl border border-border/70 bg-muted/20 p-4 text-sm sm:grid-cols-2">
        <div>
          <dt className="text-xs text-muted-foreground">Origin</dt>
          <dd className="mt-1">
            <TaskLink href={task.originatingThread}>Open conversation</TaskLink>
          </dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Parent</dt>
          <dd className="mt-1">
            <TaskLink href={task.parentChannel}>Open parent channel</TaskLink>
          </dd>
        </div>
        <div className="sm:col-span-2">
          <dt className="text-xs text-muted-foreground">
            Implementation branch
          </dt>
          <dd className="mt-1 font-mono text-xs">
            {task.branch?.name ?? "Not started"}
          </dd>
        </div>
      </dl>
    </div>
  );
}

function TaskChanges({ task }: { task: ChannelBackedTask }) {
  const branch = task.branch;
  const diff = useQuery({
    enabled: Boolean(branch),
    queryKey: ["channel-backed-task", branch?.repository, branch?.name, "diff"],
    queryFn: () =>
      getProjectRepoDiff({
        cloneUrl: branch?.repository ?? "",
        defaultBranch: branch?.name,
        baseBranch: "main",
      }),
    retry: 1,
    staleTime: 30_000,
  });
  if (!branch)
    return (
      <EmptyView
        title="No implementation branch"
        description="Changes will appear after a branch is attached to this task."
      />
    );
  if (diff.isLoading)
    return <BuzzLoadingState label="Loading branch changes" />;
  if (diff.error || !diff.data)
    return (
      <EmptyView
        title="Could not load changes"
        description={
          diff.error instanceof Error
            ? diff.error.message
            : "The branch is not available from this repository."
        }
      />
    );
  return (
    <div
      className="mx-auto w-full max-w-3xl space-y-4 px-6 py-8"
      data-testid="task-changes"
    >
      <div className="flex flex-wrap items-center gap-2">
        <GitBranch className="h-4 w-4 text-muted-foreground" />
        <code className="text-sm">{branch.name}</code>
        <span className="text-xs text-muted-foreground">against main</span>
        <span className="ml-auto text-xs">
          <span className="text-green-600">+{diff.data.additions}</span>{" "}
          <span className="text-red-600">−{diff.data.deletions}</span>
        </span>
      </div>
      <div className="divide-y overflow-hidden rounded-xl border border-border/70">
        {diff.data.files.map((file) => (
          <div
            className="flex items-center gap-3 px-4 py-3 text-sm"
            key={file.path}
          >
            <span className="min-w-0 flex-1 truncate font-mono text-xs">
              {file.path}
            </span>
            <span className="text-xs text-green-600">+{file.additions}</span>
            <span className="text-xs text-red-600">−{file.deletions}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function TaskReview({ task }: { task: ChannelBackedTask }) {
  const repository = task.branch
    ? githubRepositoryUrl(task.branch.repository)
    : null;
  const reviewsUrl =
    repository && task.branch
      ? `${repository}/pulls?q=${encodeURIComponent(`is:pr head:${task.branch.name}`)}`
      : null;
  return (
    <EmptyView
      action={
        reviewsUrl ? (
          <Button asChild size="sm" variant="outline">
            <a href={reviewsUrl} rel="noreferrer" target="_blank">
              Open GitHub reviews
            </a>
          </Button>
        ) : undefined
      }
      description="Review data is not linked to this channel yet. This view is where checks, findings, and decisions will appear."
      icon={<GitPullRequest className="h-5 w-5" />}
      title="No linked review"
    />
  );
}

function EmptyView({
  action,
  description,
  icon,
  title,
}: {
  action?: React.ReactNode;
  description: string;
  icon?: React.ReactNode;
  title: string;
}) {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center px-6 py-10 text-center">
      <div className="max-w-md space-y-3">
        {icon ? (
          <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-full bg-muted text-muted-foreground">
            {icon}
          </div>
        ) : null}
        <h2 className="text-base font-semibold">{title}</h2>
        <p className="text-sm leading-6 text-muted-foreground">{description}</p>
        {action}
      </div>
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
  const task = React.useMemo(
    () => parseChannelBackedTask(canvas.data?.content),
    [canvas.data?.content],
  );
  const [view, setView] = React.useState<TaskView>("overview");
  if (!task) return children;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <nav
        aria-label="Task views"
        className={cn(
          channelChrome.contentPadding,
          "flex gap-1 border-b border-border/70 px-5 pb-2",
        )}
        data-testid="task-view-tabs"
      >
        {VIEWS.map((item) => (
          <button
            aria-current={view === item ? "page" : undefined}
            className={cn(
              "rounded-full px-3 py-1.5 text-xs font-medium capitalize text-muted-foreground hover:bg-muted hover:text-foreground",
              view === item && "bg-muted text-foreground",
            )}
            data-testid={`task-view-${item}`}
            key={item}
            onClick={() => setView(item)}
            type="button"
          >
            {item === "conversation" ? (
              <MessagesSquare className="mr-1.5 inline h-3.5 w-3.5" />
            ) : null}
            {item}
          </button>
        ))}
      </nav>
      {view === "conversation" ? (
        children
      ) : (
        <div className="flex min-h-0 flex-1 overflow-y-auto">
          {view === "overview" ? (
            <TaskOverview task={task} />
          ) : view === "changes" ? (
            <TaskChanges task={task} />
          ) : (
            <TaskReview task={task} />
          )}
        </div>
      )}
    </div>
  );
}

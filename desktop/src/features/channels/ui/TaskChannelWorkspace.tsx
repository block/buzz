import { useQuery } from "@tanstack/react-query";
import { GitBranch, GitPullRequest, MessagesSquare } from "lucide-react";
import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";

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
import { Markdown } from "@/shared/ui/markdown";
import { invokeTauri } from "@/shared/api/tauri";

type GithubReview = {
  number: number;
  title: string;
  url: string;
  state: string;
  isDraft: boolean;
  reviewDecision: string;
  statusCheckRollup:
    | {
        name?: string;
        context?: string;
        detailsUrl?: string;
        targetUrl?: string;
        conclusion?: string;
        status?: string;
        state?: string;
      }[]
    | null;
};
type GithubChanges = {
  html_url: string;
  files: {
    filename: string;
    additions: number;
    deletions: number;
    patch?: string;
    blob_url: string;
  }[];
};

function useGithubTask(task: ChannelBackedTask, view: "changes" | "review") {
  const repository = task.branch && githubRepositoryUrl(task.branch.repository);
  return useQuery({
    queryKey: ["task-github", repository, task.branch?.name, view],
    enabled: Boolean(repository && task.branch),
    queryFn: () =>
      invokeTauri<GithubChanges | GithubReview[]>("get_task_github", {
        repository: repository?.replace("https://github.com/", ""),
        branch: task.branch?.name,
        view,
      }),
    staleTime: 30_000,
    retry: false,
  });
}

type TaskView = "overview" | "changes" | "review" | "conversation";

const VIEWS: readonly TaskView[] = [
  "overview",
  "changes",
  "review",
  "conversation",
];

function TaskLink({ href, children }: { href: string; children: string }) {
  return (
    <a
      className="text-primary hover:underline"
      href={href}
      target="_blank"
      rel="noreferrer"
      onClick={(event) => {
        event.preventDefault();
        void openUrl(href).catch(() =>
          toast.error("Could not open link in browser"),
        );
      }}
    >
      {children}
    </a>
  );
}

function TaskOverview({ task }: { task: ChannelBackedTask }) {
  const repository = task.branch && githubRepositoryUrl(task.branch.repository);
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
            <Markdown content={task.originatingThread} />
          </dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Parent</dt>
          <dd className="mt-1">
            <Markdown content={task.parentChannel} />
          </dd>
        </div>
        <div className="sm:col-span-2">
          <dt className="text-xs text-muted-foreground">
            Implementation branch
          </dt>
          <dd className="mt-1 font-mono text-xs">
            {repository && task.branch ? (
              <TaskLink
                href={`${repository}/tree/${encodeURIComponent(task.branch.name)}`}
              >
                {task.branch.name}
              </TaskLink>
            ) : (
              (task.branch?.name ?? "Not started")
            )}
          </dd>
        </div>
      </dl>
    </div>
  );
}

function TaskChanges({ task }: { task: ChannelBackedTask }) {
  const branch = task.branch;
  const github = useGithubTask(task, "changes");
  const isGithub = Boolean(branch && githubRepositoryUrl(branch.repository));
  const diff = useQuery({
    enabled: Boolean(branch) && !isGithub,
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
  if (isGithub) {
    if (github.isPending)
      return <BuzzLoadingState label="Loading GitHub changes" />;
    if (github.error)
      return (
        <EmptyView
          title="Could not load GitHub changes"
          description={github.error.message}
        />
      );
    const changes = github.data as GithubChanges;
    return (
      <div className="mx-auto w-full max-w-4xl space-y-4 p-6">
        <TaskLink href={changes.html_url}>
          {branch?.name ?? "Branch changes"}
        </TaskLink>
        {changes.files.map((file) => (
          <details key={file.filename} className="rounded border p-3" open>
            <summary className="cursor-pointer font-mono text-xs">
              {file.filename}{" "}
              <span className="text-green-600">+{file.additions}</span>{" "}
              <span className="text-red-600">−{file.deletions}</span>
            </summary>
            {file.patch ? (
              <pre className="mt-3 overflow-x-auto whitespace-pre font-mono text-xs">
                {file.patch}
              </pre>
            ) : (
              <TaskLink href={file.blob_url}>View file on GitHub</TaskLink>
            )}
          </details>
        ))}
        <p className="text-xs text-muted-foreground">
          GitHub returns patches for up to 300 files. The branch link opens the
          complete comparison.
        </p>
      </div>
    );
  }
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
  const github = useGithubTask(task, "review");
  const repository = task.branch
    ? githubRepositoryUrl(task.branch.repository)
    : null;
  const reviewsUrl =
    repository && task.branch
      ? `${repository}/pulls?q=${encodeURIComponent(`is:pr head:${task.branch.name}`)}`
      : null;
  if (repository && task.branch) {
    if (github.isPending)
      return <BuzzLoadingState label="Finding pull request" />;
    if (github.error)
      return (
        <EmptyView
          title="Could not load pull request"
          description={github.error.message}
        />
      );
    const prs = github.data as GithubReview[];
    if (!prs.length)
      return (
        <EmptyView
          title="No pull request yet"
          description={`No pull request found for ${task.branch.name}.`}
        />
      );
    const pr = prs.find((pr) => pr.state === "OPEN") ?? prs[0];
    return (
      <div className="mx-auto w-full max-w-3xl space-y-4 p-6">
        <h2 className="text-lg font-semibold">
          <TaskLink href={pr.url}>{`#${pr.number} ${pr.title}`}</TaskLink>
        </h2>
        <p className="text-sm">
          {pr.isDraft ? "Draft" : pr.state} ·{" "}
          {pr.reviewDecision || "No review decision"}
        </p>
        <ul className="space-y-2 text-sm">
          {pr.statusCheckRollup?.map((check) => (
            <li
              key={
                check.detailsUrl ??
                check.targetUrl ??
                check.name ??
                check.context
              }
            >
              {check.name ?? check.context}:{" "}
              {check.conclusion || check.state || check.status}
            </li>
          ))}
        </ul>
      </div>
    );
  }
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

import {
  ArrowLeft,
  Bot,
  Check,
  CheckCircle2,
  CircleDot,
  Copy,
  ExternalLink,
  FileDiff,
  FolderGit2,
  GitBranch,
  GitFork,
  ListTodo,
  MessageSquare,
  Users,
} from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  type Project,
  type ProjectRepoFile,
  type ProjectRepoSnapshot,
  useProjectIssuesQuery,
  useProjectQuery,
  useProjectRepoSnapshotQuery,
  useRepoStateQuery,
} from "@/features/projects/hooks";
import type { ProjectIssue } from "@/features/projects/projectIssues.mjs";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { isSafeUrl } from "@/shared/lib/url";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { UserAvatar } from "@/shared/ui/UserAvatar";

function CloneUrlRow({ url }: { url: string }) {
  const [copied, setCopied] = React.useState(false);

  const handleCopy = React.useCallback(() => {
    void navigator.clipboard.writeText(url).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2_000);
    });
  }, [url]);

  return (
    <div className="flex items-center gap-2 rounded-lg border border-border/50 bg-card/60 px-3 py-2">
      <GitFork className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      <code className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
        {url}
      </code>
      <Button
        className="h-6 w-6 shrink-0"
        onClick={handleCopy}
        size="icon"
        variant="ghost"
      >
        {copied ? (
          <Check className="h-4 w-4 text-green-500" />
        ) : (
          <Copy className="h-4 w-4" />
        )}
      </Button>
    </div>
  );
}

function ProjectStatCard({
  icon: Icon,
  label,
  value,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string | number;
}) {
  return (
    <Card className="flex items-center gap-3 border-border/50 bg-card/60 p-3 shadow-none">
      <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
      <div className="min-w-0">
        <p className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
          {label}
        </p>
        <p className="truncate text-sm font-semibold text-foreground">
          {value}
        </p>
      </div>
    </Card>
  );
}

function formatCreatedDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

function compactDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function projectPeople(project: Project, issues: ProjectIssue[]) {
  return [
    ...new Set(
      [
        project.owner,
        ...project.contributors,
        ...issues.flatMap((issue) => [issue.author, ...issue.recipients]),
      ]
        .filter(Boolean)
        .map(normalizePubkey),
    ),
  ];
}

function formatFileSize(size: number | null) {
  if (size === null) return "—";
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function baseName(path: string) {
  return path.split("/").pop() || path;
}

function dirName(path: string) {
  const index = path.lastIndexOf("/");
  return index >= 0 ? path.slice(0, index) : "/";
}

function FileBrowser({
  files,
  selectedFile,
  onSelectFile,
}: {
  files: ProjectRepoFile[];
  selectedFile: ProjectRepoFile | null;
  onSelectFile: (file: ProjectRepoFile) => void;
}) {
  if (files.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-4 text-center text-sm text-muted-foreground">
        No files have been pushed yet.
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto py-2">
      {files.slice(0, 200).map((file) => {
        const isSelected = selectedFile?.path === file.path;
        return (
          <button
            className={cn(
              "flex w-full min-w-0 items-center gap-2 px-3 py-1.5 text-left font-mono text-xs transition-colors",
              isSelected
                ? "bg-primary/10 text-foreground"
                : "text-muted-foreground hover:bg-muted/50 hover:text-foreground",
            )}
            key={file.path}
            onClick={() => onSelectFile(file)}
            type="button"
          >
            <FileDiff className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">{file.path}</span>
          </button>
        );
      })}
    </div>
  );
}

function FilePreview({ file }: { file: ProjectRepoFile | null }) {
  if (!file) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-center text-sm text-muted-foreground">
        Select a file to inspect its path and contents.
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex min-h-9 items-center gap-2 border-border/50 border-b bg-muted/20 px-3">
        <FileDiff className="h-3.5 w-3.5 text-muted-foreground" />
        <span className="truncate font-mono text-xs text-foreground">
          {baseName(file.path)}
        </span>
        <span className="ml-auto shrink-0 text-2xs text-muted-foreground">
          {formatFileSize(file.size)}
        </span>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto bg-background/60">
        {file.previewContent ? (
          <pre className="min-h-full overflow-x-auto p-4 font-mono text-xs leading-relaxed text-foreground">
            <code>{file.previewContent}</code>
          </pre>
        ) : (
          <div className="space-y-3 p-4">
            <div className="space-y-3 rounded-lg border border-border/50 bg-background/50 p-4">
              <div>
                <p className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
                  Path
                </p>
                <p className="mt-1 break-all font-mono text-sm text-foreground">
                  {file.path}
                </p>
              </div>
              <div className="grid gap-3 sm:grid-cols-3">
                <div>
                  <p className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
                    File
                  </p>
                  <p className="mt-1 truncate text-sm text-foreground">
                    {baseName(file.path)}
                  </p>
                </div>
                <div>
                  <p className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
                    Folder
                  </p>
                  <p className="mt-1 truncate font-mono text-sm text-foreground">
                    {dirName(file.path)}
                  </p>
                </div>
                <div>
                  <p className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
                    Size
                  </p>
                  <p className="mt-1 text-sm text-foreground">
                    {formatFileSize(file.size)}
                  </p>
                </div>
              </div>
            </div>
            <p className="text-sm text-muted-foreground">
              Preview unavailable for this file. Large and binary files only
              show metadata.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

function LatestCommitPanel({
  snapshot,
  isLoading,
  error,
}: {
  snapshot: ProjectRepoSnapshot | null | undefined;
  isLoading: boolean;
  error: unknown;
}) {
  const latestCommit = snapshot?.latestCommit ?? null;

  if (isLoading) {
    return <p className="p-4 text-sm text-muted-foreground">Loading commit…</p>;
  }

  if (!latestCommit) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        {error
          ? "Could not load repository activity from git."
          : "No commits are available yet."}
      </p>
    );
  }

  return (
    <div className="space-y-3 p-4">
      <div className="rounded-lg border border-border/50 bg-background/50 p-4">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="line-clamp-2 text-sm font-medium text-foreground">
              {latestCommit.subject}
            </p>
            <p className="truncate text-xs text-muted-foreground">
              {latestCommit.authorName} · {compactDate(latestCommit.timestamp)}
            </p>
          </div>
          <code className="shrink-0 rounded-md bg-muted px-2 py-1 text-xs text-muted-foreground">
            {latestCommit.shortHash}
          </code>
        </div>
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <ProjectStatCard
          icon={CircleDot}
          label="Commit"
          value={latestCommit.shortHash}
        />
        <ProjectStatCard
          icon={Users}
          label="Author"
          value={latestCommit.authorName}
        />
      </div>
    </div>
  );
}

function BranchesPanel({
  project,
  repoState,
  isLoading,
}: {
  project: Project;
  repoState: ReturnType<typeof useRepoStateQuery>["data"];
  isLoading: boolean;
}) {
  if (isLoading) {
    return (
      <p className="p-4 text-sm text-muted-foreground">Loading branches…</p>
    );
  }

  if (!repoState) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        No branch refs have been published yet.
      </p>
    );
  }

  return (
    <div className="space-y-3 p-4">
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
        <ProjectStatCard
          icon={GitBranch}
          label="Default"
          value={project.defaultBranch}
        />
        <ProjectStatCard
          icon={CircleDot}
          label="Branches"
          value={repoState.branches.length}
        />
        <ProjectStatCard
          icon={CheckCircle2}
          label="Tags"
          value={repoState.tags.length}
        />
      </div>
      <div className="space-y-1">
        {repoState.branches.slice(0, 12).map((branch) => (
          <div
            className="flex items-center justify-between gap-3 rounded-md bg-muted/30 px-3 py-1.5 text-sm"
            key={branch.name}
          >
            <span className="min-w-0 truncate font-mono">{branch.name}</span>
            <span className="shrink-0 font-mono text-xs text-muted-foreground">
              {branch.commit.slice(0, 8)}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function IssuesPanel({
  issues,
  isLoading,
}: {
  issues: ProjectIssue[];
  isLoading: boolean;
}) {
  if (isLoading) {
    return <p className="p-4 text-sm text-muted-foreground">Loading issues…</p>;
  }

  if (issues.length === 0) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        No issues yet. Git issues for this project will appear here with their
        workflow status.
      </p>
    );
  }

  return (
    <div className="space-y-2 p-4">
      {issues.slice(0, 10).map((issue) => (
        <Card
          className="space-y-2 border-border/50 bg-card/60 p-3 shadow-none"
          key={issue.id}
        >
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-medium text-foreground">
                {issue.title}
              </p>
              {issue.content ? (
                <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
                  {issue.content}
                </p>
              ) : null}
            </div>
            <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
              {issue.status}
            </span>
          </div>
          <div className="flex flex-wrap items-center gap-2 text-2xs text-muted-foreground">
            <span>Updated {compactDate(issue.updatedAt)}</span>
            {issue.labels.map((label) => (
              <span
                className="rounded-md border border-border/70 px-1.5 py-0.5"
                key={label}
              >
                {label}
              </span>
            ))}
          </div>
        </Card>
      ))}
    </div>
  );
}

function WorkspaceTabs({
  project,
  snapshot,
  snapshotError,
  snapshotLoading,
  repoState,
  repoStateLoading,
  issues,
  issuesLoading,
}: {
  project: Project;
  snapshot: ProjectRepoSnapshot | null | undefined;
  snapshotError: unknown;
  snapshotLoading: boolean;
  repoState: ReturnType<typeof useRepoStateQuery>["data"];
  repoStateLoading: boolean;
  issues: ProjectIssue[];
  issuesLoading: boolean;
}) {
  const files = snapshot?.files ?? [];
  const [selectedPath, setSelectedPath] = React.useState<string | null>(null);
  const selectedFile =
    files.find((file) => file.path === selectedPath) ?? files[0] ?? null;

  React.useEffect(() => {
    if (files.length > 0 && !files.some((file) => file.path === selectedPath)) {
      setSelectedPath(files[0].path);
    }
  }, [files, selectedPath]);

  return (
    <section className="overflow-hidden rounded-xl border border-border/50 bg-card/60 shadow-none">
      <Tabs className="flex min-h-[32rem] flex-col" defaultValue="files">
        <div className="flex items-center justify-between gap-3 border-border/50 border-b bg-muted/20 px-3 py-2">
          <div className="flex min-w-0 items-center gap-2">
            <FolderGit2 className="h-4 w-4 text-muted-foreground" />
            <span className="truncate text-sm font-medium text-foreground">
              Workspace
            </span>
          </div>
          <TabsList className="h-8">
            <TabsTrigger className="h-7 gap-1 px-2" value="files">
              <FolderGit2 className="h-3.5 w-3.5" />
              Files
            </TabsTrigger>
            <TabsTrigger className="h-7 gap-1 px-2" value="activity">
              <CircleDot className="h-3.5 w-3.5" />
              Activity
            </TabsTrigger>
            <TabsTrigger className="h-7 gap-1 px-2" value="issues">
              <ListTodo className="h-3.5 w-3.5" />
              Issues
            </TabsTrigger>
            <TabsTrigger className="h-7 gap-1 px-2" value="branches">
              <GitBranch className="h-3.5 w-3.5" />
              Branches
            </TabsTrigger>
          </TabsList>
        </div>

        <TabsContent className="m-0 min-h-0 flex-1" value="files">
          <div className="grid h-[32rem] min-h-0 grid-cols-1 lg:grid-cols-[18rem_minmax(0,1fr)]">
            <aside className="min-h-0 border-border/50 border-b bg-background/35 lg:border-r lg:border-b-0">
              <div className="flex h-8 items-center justify-between border-border/50 border-b px-3">
                <span className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
                  Explorer
                </span>
                <span className="text-2xs text-muted-foreground">
                  {files.length} files
                </span>
              </div>
              {snapshotLoading ? (
                <p className="p-3 text-sm text-muted-foreground">
                  Loading files…
                </p>
              ) : snapshotError ? (
                <p className="p-3 text-sm text-muted-foreground">
                  Could not load file tree.
                </p>
              ) : (
                <FileBrowser
                  files={files}
                  onSelectFile={(file) => setSelectedPath(file.path)}
                  selectedFile={selectedFile}
                />
              )}
            </aside>
            <FilePreview file={selectedFile} />
          </div>
        </TabsContent>

        <TabsContent className="m-0 min-h-0 flex-1" value="activity">
          <LatestCommitPanel
            error={snapshotError}
            isLoading={snapshotLoading}
            snapshot={snapshot}
          />
        </TabsContent>

        <TabsContent className="m-0 min-h-0 flex-1" value="issues">
          <IssuesPanel isLoading={issuesLoading} issues={issues} />
        </TabsContent>

        <TabsContent className="m-0 min-h-0 flex-1" value="branches">
          <BranchesPanel
            isLoading={repoStateLoading}
            project={project}
            repoState={repoState}
          />
        </TabsContent>
      </Tabs>
    </section>
  );
}

type ProjectDetailScreenProps = {
  projectId: string;
};

export function ProjectDetailScreen({ projectId }: ProjectDetailScreenProps) {
  const { goChannel, goProjects } = useAppNavigation();
  const projectQuery = useProjectQuery(projectId);
  const project = projectQuery.data;
  const repoStateQuery = useRepoStateQuery(project);
  const repoSnapshotQuery = useProjectRepoSnapshotQuery(project);
  const issuesQuery = useProjectIssuesQuery(project);
  const issues = issuesQuery.data ?? [];

  const peoplePubkeys = React.useMemo(
    () => (project ? projectPeople(project, issues) : []),
    [issues, project],
  );
  const profilesQuery = useUsersBatchQuery(peoplePubkeys, {
    enabled: peoplePubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;

  if (projectQuery.isLoading) {
    return null;
  }

  if (projectQuery.isError) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
        <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
        <p className="text-sm text-red-400">Failed to load project</p>
        <div className="flex items-center gap-2">
          <Button
            onClick={() => void projectQuery.refetch()}
            size="sm"
            variant="outline"
          >
            Retry
          </Button>
          <Button
            onClick={() => {
              void goProjects();
            }}
            size="sm"
            variant="ghost"
          >
            <ArrowLeft className="mr-1.5 h-4 w-4" />
            Back to Projects
          </Button>
        </div>
      </div>
    );
  }

  if (!project) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 px-4 py-16 text-center">
        <FolderGit2 className="h-10 w-10 text-muted-foreground/40" />
        <p className="text-sm text-muted-foreground">
          This project could not be found.
        </p>
        <Button
          onClick={() => {
            void goProjects();
          }}
          size="sm"
          variant="outline"
        >
          <ArrowLeft className="mr-1.5 h-4 w-4" />
          Back to Projects
        </Button>
      </div>
    );
  }

  const ownerProfile = profiles?.[normalizePubkey(project.owner)];
  const ownerLabel = resolveUserLabel({ pubkey: project.owner, profiles });

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div
        className={cn(
          "flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4",
          topChromeInset.padding,
        )}
      >
        <div className="mb-4">
          <Button
            className="gap-1.5 text-muted-foreground"
            onClick={() => {
              void goProjects();
            }}
            size="sm"
            variant="ghost"
          >
            <ArrowLeft className="h-4 w-4" />
            Back to Projects
          </Button>
        </div>

        <div className="mx-auto w-full max-w-5xl space-y-5">
          <section className="space-y-3 rounded-xl border border-border/50 bg-card/60 p-4">
            <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
              <div className="flex min-w-0 items-start gap-3">
                <UserAvatar
                  accent={ownerProfile?.isAgent === true}
                  avatarUrl={ownerProfile?.avatarUrl ?? null}
                  className="shrink-0"
                  displayName={ownerLabel}
                  size="md"
                />
                <div className="min-w-0 flex-1 space-y-1">
                  <div className="flex min-w-0 flex-wrap items-center gap-2">
                    <h2 className="truncate text-lg font-semibold">
                      {project.name}
                    </h2>
                    <span className="rounded-full bg-muted px-2 py-0.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
                      {project.status}
                    </span>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    Work by {ownerLabel} · Created{" "}
                    {formatCreatedDate(project.createdAt)}
                  </p>
                </div>
              </div>
              {project.projectChannelId ? (
                <Button
                  className="shrink-0 gap-1.5"
                  onClick={() => {
                    if (project.projectChannelId) {
                      void goChannel(project.projectChannelId);
                    }
                  }}
                  size="sm"
                  variant="outline"
                >
                  <MessageSquare className="h-4 w-4" />
                  Open Discussion
                </Button>
              ) : null}
            </div>
            {project.description ? (
              <p className="text-sm text-muted-foreground">
                {project.description}
              </p>
            ) : null}

            <div className="grid grid-cols-1 gap-2 sm:grid-cols-4">
              <ProjectStatCard
                icon={GitBranch}
                label="Branch"
                value={project.defaultBranch}
              />
              <ProjectStatCard
                icon={ListTodo}
                label="Issues"
                value={issues.length}
              />
              <ProjectStatCard
                icon={Users}
                label="Involved"
                value={peoplePubkeys.length}
              />
              <ProjectStatCard
                icon={MessageSquare}
                label="Discussion"
                value={project.projectChannelId ? "Linked" : "Not linked"}
              />
            </div>
          </section>

          {project.cloneUrls.length > 0 ? (
            <section className="space-y-2">
              <h3 className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Clone
              </h3>
              <div className="space-y-1.5">
                {project.cloneUrls.map((url) => (
                  <CloneUrlRow key={url} url={url} />
                ))}
              </div>
            </section>
          ) : null}

          {project.webUrl && isSafeUrl(project.webUrl) ? (
            <section className="space-y-2">
              <h3 className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                Web
              </h3>
              <a
                className="inline-flex items-center gap-1.5 text-sm text-primary hover:underline"
                href={project.webUrl}
                rel="noopener noreferrer"
                target="_blank"
              >
                <ExternalLink className="h-4 w-4" />
                {project.webUrl}
              </a>
            </section>
          ) : null}

          <WorkspaceTabs
            issues={issues}
            issuesLoading={issuesQuery.isLoading}
            project={project}
            repoState={repoStateQuery.data}
            repoStateLoading={repoStateQuery.isLoading}
            snapshot={repoSnapshotQuery.data}
            snapshotError={repoSnapshotQuery.error}
            snapshotLoading={repoSnapshotQuery.isLoading}
          />

          {peoplePubkeys.length > 0 ? (
            <section className="space-y-2">
              <h3 className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground">
                <Users className="h-4 w-4" />
                Involved ({peoplePubkeys.length})
              </h3>
              <div className="grid gap-1.5 sm:grid-cols-2">
                {peoplePubkeys.map((pubkey) => {
                  const profile = profiles?.[normalizePubkey(pubkey)];
                  const label = resolveUserLabel({ pubkey, profiles });
                  const isOwner =
                    normalizePubkey(pubkey) === normalizePubkey(project.owner);
                  return (
                    <div
                      className="flex min-w-0 items-center gap-2 rounded-lg border border-border/40 bg-card/50 px-3 py-2"
                      key={pubkey}
                    >
                      <UserAvatar
                        accent={profile?.isAgent === true || isOwner}
                        avatarUrl={profile?.avatarUrl ?? null}
                        displayName={label}
                        size="xs"
                      />
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm text-foreground">
                          {label}
                        </p>
                        <p className="truncate text-xs text-muted-foreground">
                          {isOwner ? "Project owner" : "Contributor"}
                        </p>
                      </div>
                    </div>
                  );
                })}
              </div>
            </section>
          ) : null}

          <section className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <Card className="space-y-2 border-border/50 bg-card/60 p-4 shadow-none">
              <h3 className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground">
                <Bot className="h-4 w-4" />
                Agent Work
              </h3>
              <p className="text-sm text-muted-foreground">
                Start agents from project issues so their summaries, branches,
                patches, and review notes stay attached to this project.
              </p>
            </Card>
            <Card className="space-y-2 border-border/50 bg-card/60 p-4 shadow-none">
              <h3 className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground">
                <FileDiff className="h-4 w-4" />
                Code Discussion
              </h3>
              <p className="text-sm text-muted-foreground">
                Diff messages and NIP-34 patches render in the linked discussion
                channel, giving humans and agents a shared review surface.
              </p>
            </Card>
          </section>

          <section className="space-y-2">
            <h3 className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              Details
            </h3>
            <div className="space-y-1 text-sm text-muted-foreground">
              <p className="truncate">Repo: {project.repoAddress}</p>
              <p className="truncate">
                Owner: {resolveUserLabel({ pubkey: project.owner, profiles })}
              </p>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

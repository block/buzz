import {
  ArrowLeft,
  BookOpen,
  Bot,
  Check,
  ChevronDown,
  CircleDot,
  Copy,
  ExternalLink,
  FileDiff,
  FolderGit2,
  GitBranch,
  GitFork,
  GitPullRequest,
  MessageSquare,
  Users,
} from "lucide-react";
import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  type Project,
  type ProjectPullRequest,
  type ProjectRepoContributor,
  type ProjectRepoSnapshot,
  useProjectQuery,
  useProjectPullRequestsQuery,
  useProjectRepoSnapshotQuery,
  useRepoStateQuery,
} from "@/features/projects/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { useMainInsetRef } from "@/shared/layout/MainInsetContext";
import {
  channelChrome,
  channelContentTopPaddingMeasurement,
  topChromeInset,
} from "@/shared/layout/chromeLayout";
import { useMeasuredCssVariable } from "@/shared/layout/useMeasuredCssVariable";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { isSafeUrl } from "@/shared/lib/url";
import type { ProjectRepoCommit } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  findReadmeFile,
  ReadmePanel,
  RepositoryFilesPanel,
} from "./ProjectRepositoryPanel";

function CloneUrlRow({ url }: { url: string }) {
  const [copied, setCopied] = React.useState(false);

  const handleCopy = React.useCallback(() => {
    void navigator.clipboard.writeText(url).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2_000);
    });
  }, [url]);

  return (
    <div className="flex min-w-0 items-center gap-2">
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

function RepositorySourceCard({
  branch,
  branchOptions,
  cloneUrls,
  onBranchChange,
}: {
  branch: string;
  branchOptions: string[];
  cloneUrls: string[];
  onBranchChange: (branch: string) => void;
}) {
  if (cloneUrls.length === 0 && !branch) return null;
  const selectableBranches =
    branchOptions.length > 0 ? branchOptions : [branch];

  return (
    <Card className="border-border/50 bg-card/60 p-4 shadow-none">
      <div className="flex min-w-0 flex-col">
        <div className="flex min-w-0 items-center gap-2">
          <GitBranch className="h-3.5 w-3.5 text-muted-foreground" />
          {branch ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  className="h-6 max-w-full gap-1.5 px-2 font-mono text-sm font-semibold"
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <span className="truncate">{branch || "—"}</span>
                  <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="min-w-56">
                <DropdownMenuRadioGroup
                  onValueChange={onBranchChange}
                  value={branch}
                >
                  {selectableBranches.map((option) => (
                    <DropdownMenuRadioItem key={option} value={option}>
                      <span className="truncate font-mono">{option}</span>
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : (
            <span className="truncate font-mono text-sm font-semibold text-foreground">
              {branch || "—"}
            </span>
          )}
        </div>
        <div className="min-w-0 flex-1">
          {cloneUrls.length > 0 ? (
            cloneUrls.map((url) => <CloneUrlRow key={url} url={url} />)
          ) : (
            <div className="text-sm text-muted-foreground">
              No clone URL published yet.
            </div>
          )}
        </div>
      </div>
    </Card>
  );
}

function compactDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

function projectPeople(project: Project) {
  return [
    ...new Set(
      [project.owner, ...project.contributors]
        .filter(Boolean)
        .map(normalizePubkey),
    ),
  ];
}

function contributorKey(contributor: ProjectRepoContributor) {
  return (contributor.email || contributor.name).trim().toLowerCase();
}

function profileMatchesContributor(
  contributor: ProjectRepoContributor,
  profile: UserProfileLookup[string] | undefined,
  pubkey?: string,
) {
  if (!profile) return false;
  const name = contributor.name.trim().toLowerCase();
  const email = contributor.email.trim().toLowerCase();
  const candidates = [
    pubkey,
    profile.displayName,
    profile.nip05Handle,
    profile.ownerPubkey,
  ].map((value) => value?.trim().toLowerCase() ?? "");
  return candidates.includes(name) || candidates.includes(email);
}

function contributorMatchesProfiles(
  contributor: ProjectRepoContributor,
  profiles: UserProfileLookup | undefined,
) {
  if (!profiles) return false;
  return Object.entries(profiles).some(([pubkey, profile]) =>
    profileMatchesContributor(contributor, profile, pubkey),
  );
}

function contributorForProfile(
  pubkey: string,
  profiles: UserProfileLookup | undefined,
  repoContributors: ProjectRepoContributor[],
) {
  const profile = profiles?.[normalizePubkey(pubkey)];
  return repoContributors.find((contributor) =>
    profileMatchesContributor(contributor, profile, pubkey),
  );
}

function profileForCommitAuthor(
  commit: ProjectRepoCommit,
  profiles: UserProfileLookup | undefined,
) {
  if (!profiles) return null;
  const contributor = {
    name: commit.authorName,
    email: commit.authorEmail,
    commitCount: 0,
    lastCommitAt: commit.timestamp,
  };

  for (const [pubkey, profile] of Object.entries(profiles)) {
    if (profileMatchesContributor(contributor, profile, pubkey)) {
      return { pubkey, profile };
    }
  }

  return null;
}

function ContributorsPanel({
  peoplePubkeys,
  project,
  profiles,
  repoContributors,
}: {
  peoplePubkeys: string[];
  project: Project;
  profiles?: UserProfileLookup;
  repoContributors: ProjectRepoContributor[];
}) {
  const rows = [
    ...peoplePubkeys.map((pubkey) => {
      const normalizedPubkey = normalizePubkey(pubkey);
      const profile = profiles?.[normalizedPubkey];
      const isOwner = normalizedPubkey === normalizePubkey(project.owner);
      const contributor = contributorForProfile(
        pubkey,
        profiles,
        repoContributors,
      );
      const label = resolveUserLabel({ pubkey, profiles });
      return {
        avatarUrl: profile?.avatarUrl ?? null,
        commitCount: contributor?.commitCount ?? null,
        id: `profile:${normalizedPubkey}`,
        isAgent: profile?.isAgent === true,
        label,
        lastCommitAt: contributor?.lastCommitAt ?? null,
        role: isOwner
          ? "Project owner"
          : profile?.isAgent
            ? "Agent"
            : "Contributor",
      };
    }),
    ...repoContributors
      .filter(
        (contributor) => !contributorMatchesProfiles(contributor, profiles),
      )
      .map((contributor) => ({
        avatarUrl: null,
        commitCount: contributor.commitCount,
        id: `git:${contributorKey(contributor)}`,
        isAgent: false,
        label: contributor.name || contributor.email,
        lastCommitAt: contributor.lastCommitAt,
        role: contributor.email || "Git contributor",
      })),
  ];

  if (rows.length === 0) return null;

  return (
    <section className="space-y-2">
      <h3 className="flex items-center gap-1.5 text-xs font-medium uppercase tracking-wider text-muted-foreground">
        <Users className="h-4 w-4" />
        Contributors ({rows.length})
      </h3>
      <div className="overflow-hidden rounded-xl border border-border/50 bg-card/60">
        <table className="w-full caption-bottom text-sm">
          <tbody>
            {rows.map((row, index) => (
              <tr
                className={cn(
                  "transition-colors hover:bg-muted/35",
                  index !== rows.length - 1 && "border-border/50 border-b",
                )}
                key={row.id}
              >
                <td className="min-w-52 p-3 align-middle">
                  <div className="flex min-w-0 items-center gap-2">
                    <UserAvatar
                      accent={row.isAgent}
                      avatarUrl={row.avatarUrl}
                      displayName={row.label}
                      size="xs"
                    />
                    <div className="min-w-0">
                      <p className="truncate font-medium text-foreground">
                        {row.label}
                      </p>
                      <p className="truncate text-xs text-muted-foreground">
                        {row.role}
                      </p>
                    </div>
                  </div>
                </td>
                <td className="hidden p-3 align-middle text-muted-foreground sm:table-cell">
                  {row.commitCount === null
                    ? "No git commits"
                    : `${row.commitCount} git commit${row.commitCount === 1 ? "" : "s"}`}
                </td>
                <td className="w-36 whitespace-nowrap p-3 text-right align-middle text-muted-foreground">
                  {row.lastCommitAt ? compactDate(row.lastCommitAt) : "—"}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}

function ActivityPanel({
  snapshot,
  isLoading,
  error,
  profiles,
  repoContributors,
}: {
  snapshot: ProjectRepoSnapshot | null | undefined;
  isLoading: boolean;
  error: unknown;
  profiles?: UserProfileLookup;
  repoContributors: ProjectRepoContributor[];
}) {
  const commits = snapshot?.commits ?? [];

  if (isLoading) {
    return (
      <p className="p-4 text-sm text-muted-foreground">Loading activity…</p>
    );
  }

  if (commits.length === 0) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        {error
          ? "Could not load repository activity from git."
          : "No commits are available yet."}
      </p>
    );
  }

  return (
    <div className="space-y-1 p-2">
      {commits.map((commit, index) => {
        const matchedProfile = profileForCommitAuthor(commit, profiles);
        const authorLabel = matchedProfile
          ? resolveUserLabel({ pubkey: matchedProfile.pubkey, profiles })
          : commit.authorName || commit.authorEmail || "Unknown author";
        const authorSubtitle =
          matchedProfile?.profile.nip05Handle ||
          commit.authorEmail ||
          "Git contributor";
        const matchingContributor = repoContributors.find(
          (contributor) =>
            contributor.name.trim().toLowerCase() ===
              commit.authorName.trim().toLowerCase() ||
            contributor.email.trim().toLowerCase() ===
              commit.authorEmail.trim().toLowerCase(),
        );

        return (
          <article
            className="group/feed-item relative flex min-w-0 gap-3 rounded-lg p-3 transition-colors hover:bg-muted/35"
            data-testid="project-activity-feed-item"
            key={commit.hash}
          >
            {index < commits.length - 1 ? (
              <div
                aria-hidden="true"
                className="absolute bottom-0 left-7 top-12 w-px bg-border/45"
              />
            ) : null}
            <UserAvatar
              accent={matchedProfile?.profile.isAgent === true}
              avatarUrl={matchedProfile?.profile.avatarUrl ?? null}
              className="relative z-10 mt-0.5 shrink-0 ring-2 ring-card"
              displayName={authorLabel}
              size="md"
            />
            <div className="min-w-0 flex-1 space-y-2">
              <div className="flex min-w-0 items-start gap-3">
                <div className="min-w-0 flex-1">
                  <p className="min-w-0 text-sm leading-5 text-muted-foreground">
                    <span className="font-semibold text-foreground">
                      {authorLabel}
                    </span>{" "}
                    pushed a commit
                  </p>
                  <p className="truncate text-xs text-muted-foreground/80">
                    {authorSubtitle} · {compactDate(commit.timestamp)}
                    {matchingContributor?.commitCount
                      ? ` · ${pluralize(matchingContributor.commitCount, "commit")}`
                      : ""}
                  </p>
                </div>
                <code className="shrink-0 rounded-md border border-border/50 bg-background/55 px-2 py-1 text-xs text-muted-foreground transition-colors group-hover/feed-item:text-foreground">
                  {commit.shortHash}
                </code>
              </div>
              <div className="rounded-lg border border-border/50 bg-background/45 px-3 py-2">
                <p className="line-clamp-2 text-sm font-medium leading-5 text-foreground">
                  {commit.subject}
                </p>
              </div>
            </div>
          </article>
        );
      })}
    </div>
  );
}

function PullRequestsPanel({
  error,
  isLoading,
  pullRequests,
}: {
  error: unknown;
  isLoading: boolean;
  pullRequests: ProjectPullRequest[];
}) {
  if (isLoading) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        Loading pull requests…
      </p>
    );
  }

  if (pullRequests.length === 0) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        {error
          ? "Could not load pull requests for this repository."
          : "No pull requests yet."}
      </p>
    );
  }

  return (
    <div className="divide-y divide-border/50">
      {pullRequests.map((pullRequest) => (
        <article
          className="flex min-w-0 items-start gap-3 p-3 transition-colors hover:bg-muted/30"
          key={pullRequest.id}
        >
          <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <GitPullRequest className="h-4 w-4" />
          </div>
          <div className="min-w-0 flex-1 space-y-1">
            <p className="truncate text-sm font-medium text-foreground">
              {pullRequest.title}
            </p>
            <p className="truncate text-xs text-muted-foreground">
              {pullRequest.branchName ? `${pullRequest.branchName} · ` : ""}
              {pullRequest.updateCount > 0
                ? `${pluralize(pullRequest.updateCount, "update")} · `
                : ""}
              {compactDate(pullRequest.updatedAt)}
            </p>
            {pullRequest.content ? (
              <p className="line-clamp-2 text-sm text-muted-foreground">
                {pullRequest.content}
              </p>
            ) : null}
          </div>
          {pullRequest.commit ? (
            <code className="shrink-0 rounded-md bg-background/55 px-2 py-1 text-xs text-muted-foreground">
              {pullRequest.commit.slice(0, 7)}
            </code>
          ) : null}
        </article>
      ))}
    </div>
  );
}

function WorkspaceTabs({
  project,
  pullRequests,
  pullRequestsError,
  pullRequestsLoading,
  snapshot,
  snapshotError,
  snapshotLoading,
  profiles,
  peoplePubkeys,
  repoContributors,
}: {
  project: Project;
  pullRequests: ProjectPullRequest[];
  pullRequestsError: unknown;
  pullRequestsLoading: boolean;
  snapshot: ProjectRepoSnapshot | null | undefined;
  snapshotError: unknown;
  snapshotLoading: boolean;
  profiles?: UserProfileLookup;
  peoplePubkeys: string[];
  repoContributors: ProjectRepoContributor[];
}) {
  const files = snapshot?.files ?? [];
  const readmeFile = React.useMemo(() => findReadmeFile(files), [files]);
  const [selectedTab, setSelectedTab] = React.useState("readme");

  React.useEffect(() => {
    setSelectedTab((currentTab) =>
      currentTab === "readme" && !readmeFile && !snapshotLoading
        ? "activity"
        : currentTab,
    );
  }, [readmeFile, snapshotLoading]);

  return (
    <Tabs
      className="space-y-3"
      onValueChange={setSelectedTab}
      value={selectedTab}
    >
      <TabsList className="h-8 w-fit justify-start">
        {readmeFile ? (
          <TabsTrigger
            aria-label="README"
            className="h-7 px-2"
            title="README"
            value="readme"
          >
            <BookOpen className="h-3.5 w-3.5" />
          </TabsTrigger>
        ) : null}
        <TabsTrigger className="h-7 gap-1 px-2" value="activity">
          <CircleDot className="h-3.5 w-3.5" />
          Activity
        </TabsTrigger>
        <TabsTrigger className="h-7 gap-1 px-2" value="prs">
          <GitPullRequest className="h-3.5 w-3.5" />
          PRs
        </TabsTrigger>
        <TabsTrigger className="h-7 gap-1 px-2" value="files">
          <FolderGit2 className="h-3.5 w-3.5" />
          Files
        </TabsTrigger>
        <TabsTrigger className="h-7 gap-1 px-2" value="contributors">
          <Users className="h-3.5 w-3.5" />
          Contributors
        </TabsTrigger>
      </TabsList>

      <TabsContent
        className="m-0 overflow-hidden rounded-xl border border-border/50 bg-card/60"
        value="activity"
      >
        <ActivityPanel
          error={snapshotError}
          isLoading={snapshotLoading}
          profiles={profiles}
          repoContributors={repoContributors}
          snapshot={snapshot}
        />
      </TabsContent>

      <TabsContent
        className="m-0 overflow-hidden rounded-xl border border-border/50 bg-card/60"
        value="prs"
      >
        <PullRequestsPanel
          error={pullRequestsError}
          isLoading={pullRequestsLoading}
          pullRequests={pullRequests}
        />
      </TabsContent>

      <TabsContent className="m-0" value="files">
        <RepositoryFilesPanel
          error={snapshotError}
          fallbackAuthorPubkey={project.owner}
          files={files}
          isLoading={snapshotLoading}
          profiles={profiles}
          snapshot={snapshot}
        />
      </TabsContent>

      {readmeFile ? (
        <TabsContent className="m-0" value="readme">
          <ReadmePanel file={readmeFile} />
        </TabsContent>
      ) : null}

      <TabsContent className="m-0" value="contributors">
        <ContributorsPanel
          peoplePubkeys={peoplePubkeys}
          profiles={profiles}
          project={project}
          repoContributors={repoContributors}
        />
      </TabsContent>
    </Tabs>
  );
}

type ProjectDetailScreenProps = {
  projectId: string;
};

export function ProjectDetailScreen({ projectId }: ProjectDetailScreenProps) {
  const { goChannel, goProjects } = useAppNavigation();
  const mainInsetRef = useMainInsetRef();
  const projectDetailHeaderChromeRef = useMeasuredCssVariable({
    targetRef: mainInsetRef,
    resetKey: projectId,
    ...channelContentTopPaddingMeasurement,
  });
  const projectQuery = useProjectQuery(projectId);
  const project = projectQuery.data;
  const repoStateQuery = useRepoStateQuery(project);
  const branchOptions = React.useMemo(() => {
    const names = [
      project?.defaultBranch,
      ...(repoStateQuery.data?.branches.map((branch) => branch.name) ?? []),
    ].filter((name): name is string => Boolean(name));
    return [...new Set(names)];
  }, [project?.defaultBranch, repoStateQuery.data?.branches]);
  const [selectedBranch, setSelectedBranch] = React.useState<string | null>(
    null,
  );
  const activeBranch =
    selectedBranch ?? project?.defaultBranch ?? branchOptions[0] ?? null;
  const repoSnapshotQuery = useProjectRepoSnapshotQuery(project, activeBranch);
  const pullRequestsQuery = useProjectPullRequestsQuery(project);

  React.useEffect(() => {
    if (!project) {
      setSelectedBranch(null);
      return;
    }

    setSelectedBranch((currentBranch) => {
      if (currentBranch && branchOptions.includes(currentBranch)) {
        return currentBranch;
      }
      return project.defaultBranch ?? branchOptions[0] ?? null;
    });
  }, [project, branchOptions]);

  const peoplePubkeys = React.useMemo(
    () => (project ? projectPeople(project) : []),
    [project],
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
  const repoContributors = repoSnapshotQuery.data?.contributors ?? [];

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div
        className={cn(
          "pointer-events-none relative z-30 overflow-hidden rounded-tl-xl bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
          channelChrome.negativeMargin,
          topChromeInset.divider,
        )}
        ref={projectDetailHeaderChromeRef}
      >
        <div
          className="pointer-events-auto flex min-h-[3.25rem] items-center justify-between gap-3 px-5 py-2"
          data-tauri-drag-region
        >
          <Button
            className="h-9 gap-1.5 text-muted-foreground"
            onClick={() => {
              void goProjects();
            }}
            size="sm"
            variant="ghost"
          >
            <ArrowLeft className="h-4 w-4" />
            Back to Projects
          </Button>
          {project.projectChannelId ? (
            <Button
              className="h-9 shrink-0 gap-1.5"
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
      </div>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto px-4 pb-4">
        <div className="w-full space-y-5 pt-[calc(var(--buzz-channel-content-top-padding,5.75rem)_+_1px)]">
          <section className="space-y-3 rounded-xl border border-border/50 bg-card/60 p-4">
            <div className="flex min-w-0 items-start gap-3">
              <UserAvatar
                accent={ownerProfile?.isAgent === true}
                avatarUrl={ownerProfile?.avatarUrl ?? null}
                className="shrink-0"
                displayName={ownerLabel}
                size="md"
              />
              <div className="min-w-0 flex-1 space-y-1">
                <h2 className="truncate text-lg font-semibold">
                  {project.name}
                </h2>
                {project.description ? (
                  <p className="text-sm text-muted-foreground">
                    {project.description}
                  </p>
                ) : null}
              </div>
            </div>

            <RepositorySourceCard
              branch={activeBranch ?? ""}
              branchOptions={branchOptions}
              cloneUrls={project.cloneUrls}
              onBranchChange={setSelectedBranch}
            />
          </section>

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
            key={project.id}
            peoplePubkeys={peoplePubkeys}
            profiles={profiles}
            project={project}
            pullRequests={pullRequestsQuery.data ?? []}
            pullRequestsError={pullRequestsQuery.error}
            pullRequestsLoading={pullRequestsQuery.isLoading}
            repoContributors={repoContributors}
            snapshot={repoSnapshotQuery.data}
            snapshotError={repoSnapshotQuery.error}
            snapshotLoading={repoSnapshotQuery.isLoading}
          />

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

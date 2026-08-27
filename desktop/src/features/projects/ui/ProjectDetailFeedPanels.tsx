import {
  commitAuthorPubkeysFromPullRequests,
  contributorKey,
  profileForCommit,
  profileForContributor,
  type ProjectContributorActivityCounts,
  type ViewerGitIdentity,
} from "@/features/projects/lib/projectContributorMatching";
import type {
  ProjectPullRequest,
  ProjectRepoContributor,
  ProjectRepoSnapshot,
  Repository,
} from "@/features/projects/hooks";
import { selectionItemFromCommit } from "@/features/projects/lib/projectSelection";
import { commitShareLink } from "@/features/projects/lib/projectShareLinks";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import type { ProjectRepoCommit } from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import {
  CircleDot,
  FolderGit2,
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
} from "lucide-react";

import { CopyCommitHashButton } from "./ProjectCommitCopyButton";
import { PROJECT_DETAIL_PANEL_CLASS } from "./projectPanelStyles";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";
import { ProjectWorkItemRow } from "./ProjectWorkItemRow";
import { ProjectPanelState } from "./ProjectPanelState";

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

function commitSelectionItem(
  commit: ProjectRepoCommit,
  repository: Repository,
  projectId: string,
  author?: string | null,
) {
  return selectionItemFromCommit({
    author,
    channelId: repository.channelId,
    commitHash: commit.hash,
    projectId,
    shareLink: commitShareLink(repository, commit.hash),
    title: commit.subject,
  });
}

export function ContributorsPanel({
  activityCounts,
  contributorPubkeys,
  contributorPubkeysByGitIdentity,
  profiles,
  repoContributors,
}: {
  activityCounts: Record<string, ProjectContributorActivityCounts>;
  contributorPubkeys: string[];
  contributorPubkeysByGitIdentity: ReadonlyMap<string, string>;
  profiles?: UserProfileLookup;
  repoContributors: ProjectRepoContributor[];
}) {
  const gitRows = repoContributors.map((contributor) => {
    const signedPubkey = contributorPubkeysByGitIdentity.get(
      contributorKey(contributor),
    );
    const signedProfile = signedPubkey ? profiles?.[signedPubkey] : undefined;
    const heuristicProfile = signedPubkey
      ? null
      : profileForContributor(contributor, profiles);
    const matchedPubkey = signedPubkey ?? heuristicProfile?.pubkey ?? null;
    const matchedProfile = signedProfile ?? heuristicProfile?.profile;
    const label = matchedProfile
      ? resolveUserLabel({ pubkey: matchedPubkey ?? "", profiles })
      : contributor.name || contributor.email || "Unknown contributor";
    const signedCounts = matchedPubkey
      ? activityCounts[matchedPubkey]
      : undefined;

    return {
      avatarUrl: matchedProfile?.avatarUrl ?? null,
      commitCount: contributor.commitCount,
      id: `git:${contributorKey(contributor)}`,
      isAgent: matchedProfile?.isAgent === true,
      label,
      pubkey: matchedPubkey,
      profileLinked: matchedPubkey !== null,
      reviewCount: signedCounts?.reviews ?? null,
      role: signedPubkey
        ? matchedProfile?.nip05Handle || contributor.email || "Buzz contributor"
        : heuristicProfile
          ? `${
              heuristicProfile.profile.nip05Handle ||
              contributor.email ||
              "Git contributor"
            } · unverified match`
          : contributor.email || "Git contributor",
      taskCount: signedCounts?.tasks ?? null,
    };
  });
  const matchedPubkeys = new Set(
    gitRows
      .map((row) => row.pubkey)
      .filter((pubkey): pubkey is string => pubkey !== null),
  );
  const linkedRows = contributorPubkeys
    .filter((pubkey) => !matchedPubkeys.has(pubkey))
    .map((pubkey) => {
      const profile = profiles?.[pubkey];
      const isAgent = profile?.isAgent === true;
      const signedCounts = activityCounts[pubkey] ?? {
        commits: 0,
        reviews: 0,
        tasks: 0,
      };
      return {
        avatarUrl: profile?.avatarUrl ?? null,
        commitCount: signedCounts.commits,
        id: `buzz:${pubkey}`,
        isAgent,
        label: profile
          ? resolveUserLabel({ profiles, pubkey })
          : truncatePubkey(pubkey),
        profileLinked: true,
        pubkey,
        reviewCount: signedCounts.reviews,
        role:
          profile?.nip05Handle ||
          (isAgent ? "Agent contributor" : "Buzz contributor"),
        taskCount: signedCounts.tasks,
      };
    });
  const rows = [
    ...gitRows.filter((row) => row.profileLinked),
    ...linkedRows,
    ...gitRows.filter((row) => !row.profileLinked),
  ].sort(
    (left, right) =>
      (right.commitCount ?? -1) - (left.commitCount ?? -1) ||
      left.label.localeCompare(right.label),
  );

  if (rows.length === 0) {
    return (
      <ProjectPanelState
        description="Contributors appear after signed project or repository activity."
        title="No contributors yet"
      />
    );
  }

  return (
    <div
      className={`${PROJECT_DETAIL_PANEL_CLASS} mx-4`}
      data-project-detail-panel
    >
      {rows.map((row) => (
        <div
          className="flex min-h-9 min-w-0 items-center gap-2 px-4 py-1.5 transition-colors hover:bg-muted/35"
          data-project-contributor-kind={row.isAgent ? "agent" : "human"}
          data-testid="project-contributor-row"
          key={row.id}
        >
          <ProfileIdentityButton
            avatarClassName="shrink-0"
            avatarSize="xs"
            avatarUrl={row.avatarUrl}
            isAgent={row.isAgent}
            label={row.label}
            pubkey={row.pubkey}
            showLabel={false}
          />
          <span
            className="min-w-0 flex-1 truncate text-sm font-medium text-foreground"
            data-projects-text-priority="primary"
            title={row.label}
          >
            {row.label}
          </span>
          <span
            className="hidden min-w-0 flex-1 truncate text-xs text-muted-foreground md:block"
            data-testid="project-contributor-identity"
            title={row.role}
          >
            {row.role}
          </span>
          <span
            className="flex w-14 shrink-0 items-center justify-end gap-1 text-xs tabular-nums text-muted-foreground"
            data-testid="project-contributor-commit-count"
            title={
              row.commitCount === null
                ? "No git commits"
                : pluralize(row.commitCount, "commit")
            }
          >
            <GitCommitHorizontal className="h-3.5 w-3.5" />
            {row.commitCount ?? 0}
          </span>
          <span
            className="flex w-14 shrink-0 items-center justify-end gap-1 text-xs tabular-nums text-muted-foreground"
            data-testid="project-contributor-review-count"
            title={
              row.reviewCount === null
                ? "No linked reviews"
                : pluralize(row.reviewCount, "review")
            }
          >
            <GitPullRequest className="h-3.5 w-3.5" />
            {row.reviewCount ?? 0}
          </span>
          <span
            className="flex w-14 shrink-0 items-center justify-end gap-1 text-xs tabular-nums text-muted-foreground"
            data-testid="project-contributor-task-count"
            title={
              row.taskCount === null
                ? "No linked tasks"
                : pluralize(row.taskCount, "task")
            }
          >
            <CircleDot className="h-3.5 w-3.5" />
            {row.taskCount ?? 0}
          </span>
        </div>
      ))}
    </div>
  );
}

export function ActivityPanel({
  branch,
  commitItems,
  snapshot,
  isLoading,
  error,
  onSelectCommit,
  profiles,
  project,
  projectId,
  pullRequests,
  repoContributors,
  viewerGitIdentity,
}: {
  branch?: string;
  commitItems?: Array<{
    branch?: string;
    commit: ProjectRepoCommit;
    project: Repository;
    projectId: string;
    pullRequests?: ProjectPullRequest[];
    repoContributors?: ProjectRepoContributor[];
  }>;
  snapshot: ProjectRepoSnapshot | null | undefined;
  isLoading: boolean;
  error: unknown;
  onSelectCommit?: (commit: ProjectRepoCommit, project: Repository) => void;
  profiles?: UserProfileLookup;
  project: Repository;
  projectId: string;
  pullRequests?: ProjectPullRequest[];
  repoContributors: ProjectRepoContributor[];
  viewerGitIdentity?: ViewerGitIdentity | null;
}) {
  const items =
    commitItems ??
    (snapshot?.commits ?? []).map((commit) => ({
      branch,
      commit,
      project,
      projectId,
      pullRequests,
      repoContributors,
    }));
  const showRepositoryName =
    commitItems !== undefined &&
    new Set(items.map((item) => item.project.repoAddress)).size > 1;
  const rangeItems = items.map((item) => {
    const commitAuthorPubkeys = commitAuthorPubkeysFromPullRequests(
      item.pullRequests ?? [],
    );
    const matchedProfile = profileForCommit(
      item.commit,
      profiles,
      commitAuthorPubkeys,
      viewerGitIdentity,
    );
    return commitSelectionItem(
      item.commit,
      item.project,
      item.projectId,
      matchedProfile?.pubkey,
    );
  });

  if (isLoading) {
    return <BuzzLoadingState label="Loading activity" />;
  }

  if (items.length === 0) {
    return (
      <ProjectPanelState
        description={
          error
            ? "Refresh the repository and try again."
            : commitItems
              ? "Commits pushed to this project's repositories will appear here."
              : "Commits pushed to this repository will appear here."
        }
        error={Boolean(error)}
        title={error ? "Could not load commits" : "No commits yet"}
      />
    );
  }

  return (
    <section className={PROJECT_DETAIL_PANEL_CLASS} data-project-detail-panel>
      <div className="space-y-0.5 px-2">
        {items.map((item) => {
          const commitAuthorPubkeys = commitAuthorPubkeysFromPullRequests(
            item.pullRequests ?? [],
          );
          const matchedProfile = profileForCommit(
            item.commit,
            profiles,
            commitAuthorPubkeys,
            viewerGitIdentity,
          );
          const authorLabel = matchedProfile
            ? resolveUserLabel({
                pubkey: matchedProfile.pubkey,
                profiles,
              })
            : item.commit.authorName ||
              item.commit.authorEmail ||
              "Unknown author";
          const matchingContributor = (item.repoContributors ?? []).find(
            (contributor) =>
              contributor.name.trim().toLowerCase() ===
                item.commit.authorName.trim().toLowerCase() ||
              contributor.email.trim().toLowerCase() ===
                item.commit.authorEmail.trim().toLowerCase(),
          );

          return (
            <ProjectWorkItemRow
              eventId={item.commit.hash}
              identifier={item.commit.shortHash}
              identifierClassName="font-mono"
              identifierTitle={`View commit ${item.commit.shortHash}`}
              key={`${item.project.repoAddress}:${item.commit.hash}`}
              metadata={
                showRepositoryName || item.branch ? (
                  <span className="inline-flex min-w-0 items-center gap-1">
                    {showRepositoryName ? (
                      <>
                        <FolderGit2 className="h-3 w-3 shrink-0" />
                        <span className="truncate">{item.project.name}</span>
                      </>
                    ) : (
                      <>
                        <GitBranch className="h-3 w-3 shrink-0" />
                        <span className="truncate">{item.branch}</span>
                      </>
                    )}
                  </span>
                ) : undefined
              }
              onOpen={
                onSelectCommit
                  ? () => onSelectCommit(item.commit, item.project)
                  : undefined
              }
              selection={{
                item: commitSelectionItem(
                  item.commit,
                  item.project,
                  item.projectId,
                  matchedProfile?.pubkey,
                ),
                rangeItems,
              }}
              statusIcon={
                <GitCommitHorizontal className="h-3.5 w-3.5 text-muted-foreground/70" />
              }
              testId="project-activity-feed-item"
              title={item.commit.subject}
              trailing={
                <>
                  <span
                    className="flex h-5 w-5 shrink-0 items-center justify-center"
                    data-testid="project-commit-author"
                    title={`Committed by ${authorLabel}${
                      matchingContributor?.commitCount
                        ? ` · ${pluralize(
                            matchingContributor.commitCount,
                            "commit",
                          )}`
                        : ""
                    }`}
                  >
                    <ProfileIdentityButton
                      avatarClassName="shrink-0"
                      avatarSize="xs"
                      avatarUrl={matchedProfile?.profile.avatarUrl ?? null}
                      isAgent={matchedProfile?.profile.isAgent === true}
                      label={authorLabel}
                      pubkey={matchedProfile?.pubkey ?? null}
                      showLabel={false}
                    />
                  </span>
                  <CopyCommitHashButton
                    className="h-5 w-5 shrink-0 text-muted-foreground/60"
                    hash={item.commit.hash}
                  />
                  <span
                    className="hidden w-20 shrink-0 whitespace-nowrap text-right text-xs text-muted-foreground/55 sm:block"
                    data-testid="project-commit-row-date"
                    title={new Date(
                      item.commit.timestamp * 1_000,
                    ).toLocaleString()}
                  >
                    {relativeTime(item.commit.timestamp)}
                  </span>
                </>
              }
            />
          );
        })}
      </div>
    </section>
  );
}

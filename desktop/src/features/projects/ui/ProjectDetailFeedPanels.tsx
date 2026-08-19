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
} from "@/features/projects/hooks";
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
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
} from "lucide-react";

import { CopyCommitHashButton } from "./ProjectCommitCopyButton";
import {
  PROJECT_DETAIL_PANEL_CLASS,
  PROJECT_DETAIL_PANEL_MESSAGE_CLASS,
} from "./projectPanelStyles";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";
import { ProjectWorkItemRow } from "./ProjectWorkItemRow";

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
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
      <p
        className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}
        data-project-detail-panel
      >
        No git contributors are available yet.
      </p>
    );
  }

  return (
    <div className={PROJECT_DETAIL_PANEL_CLASS} data-project-detail-panel>
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
  snapshot,
  isLoading,
  error,
  onSelectCommit,
  profiles,
  pullRequests,
  repoContributors,
  viewerGitIdentity,
}: {
  branch?: string;
  snapshot: ProjectRepoSnapshot | null | undefined;
  isLoading: boolean;
  error: unknown;
  onSelectCommit?: (commit: ProjectRepoCommit) => void;
  profiles?: UserProfileLookup;
  pullRequests?: ProjectPullRequest[];
  repoContributors: ProjectRepoContributor[];
  viewerGitIdentity?: ViewerGitIdentity | null;
}) {
  const commits = snapshot?.commits ?? [];
  const commitAuthorPubkeys = commitAuthorPubkeysFromPullRequests(
    pullRequests ?? [],
  );

  if (isLoading) {
    return <BuzzLoadingState label="Loading activity" />;
  }

  if (commits.length === 0) {
    return (
      <p
        className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}
        data-project-detail-panel
      >
        {error
          ? "Could not load repository activity from git."
          : "No commits are available yet."}
      </p>
    );
  }

  return (
    <section className={PROJECT_DETAIL_PANEL_CLASS} data-project-detail-panel>
      <div className="space-y-0.5 px-2">
        {commits.map((commit) => {
          const matchedProfile = profileForCommit(
            commit,
            profiles,
            commitAuthorPubkeys,
            viewerGitIdentity,
          );
          const authorLabel = matchedProfile
            ? resolveUserLabel({
                pubkey: matchedProfile.pubkey,
                profiles,
              })
            : commit.authorName || commit.authorEmail || "Unknown author";
          const matchingContributor = repoContributors.find(
            (contributor) =>
              contributor.name.trim().toLowerCase() ===
                commit.authorName.trim().toLowerCase() ||
              contributor.email.trim().toLowerCase() ===
                commit.authorEmail.trim().toLowerCase(),
          );

          return (
            <ProjectWorkItemRow
              eventId={commit.hash}
              identifier={commit.shortHash}
              identifierClassName="font-mono"
              identifierTitle={`View commit ${commit.shortHash}`}
              key={commit.hash}
              metadata={
                branch ? (
                  <span className="inline-flex min-w-0 items-center gap-1">
                    <GitBranch className="h-3 w-3 shrink-0" />
                    <span className="truncate">{branch}</span>
                  </span>
                ) : undefined
              }
              onOpen={onSelectCommit ? () => onSelectCommit(commit) : undefined}
              statusIcon={
                <GitCommitHorizontal className="h-3.5 w-3.5 text-muted-foreground/70" />
              }
              testId="project-activity-feed-item"
              title={commit.subject}
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
                    className="h-5 w-5 shrink-0 text-muted-foreground/70"
                    hash={commit.hash}
                  />
                  <span
                    className="hidden w-20 shrink-0 whitespace-nowrap text-right text-xs text-muted-foreground/70 sm:block"
                    data-testid="project-commit-row-date"
                    title={new Date(commit.timestamp * 1_000).toLocaleString()}
                  >
                    {relativeTime(commit.timestamp)}
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

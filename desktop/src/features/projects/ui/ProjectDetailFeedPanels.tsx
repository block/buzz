import {
  commitAuthorPubkeysFromPullRequests,
  contributorKey,
  profileForCommit,
  profileForContributor,
  type ViewerGitIdentity,
} from "@/features/projects/lib/projectContributorMatching";
import type {
  ProjectPullRequest,
  ProjectRepoContributor,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import type { ProjectRepoCommit } from "@/shared/api/types";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { GitBranch } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { CopyCommitHashButton } from "./ProjectCommitCopyButton";
import {
  ProjectFeedRow,
  ProjectFeedRowCluster,
  ProjectFeedRowMonoCell,
} from "./ProjectFeedRow";
import {
  PROJECT_DETAIL_PANEL_CLASS,
  PROJECT_DETAIL_PANEL_MESSAGE_CLASS,
} from "./projectPanelStyles";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

export function ContributorsPanel({
  profiles,
  repoContributors,
}: {
  profiles?: UserProfileLookup;
  repoContributors: ProjectRepoContributor[];
}) {
  const rows = repoContributors.map((contributor) => {
    const matchedProfile = profileForContributor(contributor, profiles);
    const label = matchedProfile
      ? resolveUserLabel({ pubkey: matchedProfile.pubkey, profiles })
      : contributor.name || contributor.email || "Unknown contributor";

    return {
      avatarUrl: matchedProfile?.profile.avatarUrl ?? null,
      commitCount: contributor.commitCount,
      id: `git:${contributorKey(contributor)}`,
      isAgent: matchedProfile?.profile.isAgent === true,
      label,
      lastCommitAt: contributor.lastCommitAt,
      pubkey: matchedProfile?.pubkey ?? null,
      // Profile matches come from unauthenticated git author strings, so
      // they are surfaced as unverified rather than as a confirmed identity.
      role: matchedProfile
        ? `${
            matchedProfile.profile.nip05Handle ||
            contributor.email ||
            "Git contributor"
          } · unverified match`
        : contributor.email || "Git contributor",
    };
  });

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
      {rows.map((row, index) => (
        <div
          className={cn(
            "flex min-w-0 items-center gap-3 p-3 transition-colors hover:bg-muted/35",
            index !== rows.length - 1 && "border-border/50 border-b",
          )}
          data-testid="project-contributor-row"
          key={row.id}
        >
          <ProfileIdentityButton
            avatarClassName="shrink-0"
            avatarSize="md"
            avatarUrl={row.avatarUrl}
            isAgent={row.isAgent}
            label={row.label}
            pubkey={row.pubkey}
            showLabel={false}
          />
          <div className="min-w-0 flex-1 space-y-0.5">
            <p className="truncate text-sm font-semibold leading-5 text-foreground">
              {row.label}
            </p>
            <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-4 text-muted-foreground">
              <span className="truncate">{row.role}</span>
              <span className="rounded-full border border-border/60 px-1.5 py-0.5 text-2xs">
                {row.commitCount === null
                  ? "No git commits"
                  : `${row.commitCount} commit${row.commitCount === 1 ? "" : "s"}`}
              </span>
            </div>
          </div>
          {row.lastCommitAt ? (
            <span
              className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground sm:block"
              data-testid="project-contributor-row-date"
              title={new Date(row.lastCommitAt * 1_000).toLocaleString()}
            >
              {relativeTime(row.lastCommitAt)}
            </span>
          ) : null}
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
    return (
      <p
        className={PROJECT_DETAIL_PANEL_MESSAGE_CLASS}
        data-project-detail-panel
      >
        Loading activity…
      </p>
    );
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
      <div className="divide-y divide-border/50">
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
            <ProjectFeedRow
              key={commit.hash}
              meta={
                <>
                  <ProfileIdentityButton
                    avatarClassName="shrink-0"
                    avatarSize="xs"
                    avatarUrl={matchedProfile?.profile.avatarUrl ?? null}
                    isAgent={matchedProfile?.profile.isAgent === true}
                    label={authorLabel}
                    pubkey={matchedProfile?.pubkey ?? null}
                    showLabel={false}
                  />
                  <span className="truncate">
                    <span className="font-medium text-foreground/80">
                      {authorLabel}
                    </span>{" "}
                    committed
                  </span>
                  {branch ? (
                    <span className="inline-flex min-w-0 items-center gap-1 rounded-full border border-border/60 px-1.5 py-0.5 font-mono text-2xs">
                      <GitBranch className="h-3 w-3 shrink-0" />
                      <span className="truncate">{branch}</span>
                    </span>
                  ) : null}
                  {matchingContributor?.commitCount ? (
                    <span className="rounded-full border border-border/60 px-1.5 py-0.5 text-2xs">
                      {pluralize(matchingContributor.commitCount, "commit")}
                    </span>
                  ) : null}
                </>
              }
              onOpen={onSelectCommit ? () => onSelectCommit(commit) : undefined}
              testId="project-activity-feed-item"
              title={commit.subject}
              trailing={
                <>
                  <ProjectFeedRowCluster>
                    <ProjectFeedRowMonoCell
                      label={commit.shortHash}
                      onClick={
                        onSelectCommit
                          ? () => onSelectCommit(commit)
                          : undefined
                      }
                      title={`View commit ${commit.shortHash}`}
                    />
                    <CopyCommitHashButton hash={commit.hash} />
                  </ProjectFeedRowCluster>
                  <span
                    className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground sm:block"
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

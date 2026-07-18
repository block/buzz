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
import type { ProjectRepoCommit } from "@/shared/api/types";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { GitBranch, GitCommitHorizontal } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { CopyCommitHashButton } from "./ProjectCommitCopyButton";
import {
  ProjectFeedRow,
  ProjectFeedRowCluster,
  ProjectFeedRowMonoCell,
} from "./ProjectFeedRow";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";

function compactDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function relativeCommitTime(createdAt: number) {
  const elapsedSeconds = Math.max(
    1,
    Math.floor(Date.now() / 1_000 - createdAt),
  );
  const units = [
    { label: "year", seconds: 365 * 24 * 60 * 60 },
    { label: "month", seconds: 30 * 24 * 60 * 60 },
    { label: "day", seconds: 24 * 60 * 60 },
    { label: "hour", seconds: 60 * 60 },
    { label: "min", seconds: 60 },
  ];
  const unit =
    units.find((item) => elapsedSeconds >= item.seconds) ??
    units[units.length - 1];
  const value = Math.max(1, Math.floor(elapsedSeconds / unit.seconds));
  return `${value} ${unit.label}${value === 1 ? "" : "s"} ago`;
}

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
      <p className="rounded-xl border border-border/60 bg-card p-4 text-sm text-muted-foreground">
        No git contributors are available yet.
      </p>
    );
  }

  return (
    <div className="overflow-hidden rounded-xl border border-border/60 bg-card">
      {rows.map((row, index) => (
        <div
          className={cn(
            "flex min-w-0 items-start gap-3 p-3 transition-colors hover:bg-muted/35",
            index !== rows.length - 1 && "border-border/50 border-b",
          )}
          key={row.id}
        >
          <ProfileIdentityButton
            avatarClassName="mt-0.5 shrink-0"
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
              {row.lastCommitAt ? (
                <>
                  <span>·</span>
                  <span>updated {compactDate(row.lastCommitAt)}</span>
                </>
              ) : null}
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function commitDayLabel(timestamp: number) {
  return new Date(timestamp * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

function groupCommitsByDay(commits: ProjectRepoCommit[]) {
  const groups: { label: string; commits: ProjectRepoCommit[] }[] = [];
  for (const commit of commits) {
    const label = commitDayLabel(commit.timestamp);
    const lastGroup = groups[groups.length - 1];
    if (lastGroup && lastGroup.label === label) {
      lastGroup.commits.push(commit);
    } else {
      groups.push({ label, commits: [commit] });
    }
  }
  return groups;
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
      <p className="rounded-xl border border-border/60 bg-card p-4 text-sm text-muted-foreground">
        Loading activity…
      </p>
    );
  }

  if (commits.length === 0) {
    return (
      <p className="rounded-xl border border-border/60 bg-card p-4 text-sm text-muted-foreground">
        {error
          ? "Could not load repository activity from git."
          : "No commits are available yet."}
      </p>
    );
  }

  const groups = groupCommitsByDay(commits);

  return (
    <div className="relative space-y-5">
      {/* Timeline rail connecting the date group markers, GitHub-style. */}
      <div
        aria-hidden="true"
        className="absolute bottom-2 left-[7px] top-2 w-px bg-border/45"
      />
      {groups.map((group) => (
        <section className="relative pl-7" key={group.label}>
          <span className="absolute left-0 top-0.5 flex h-4 w-4 items-center justify-center rounded-full bg-background ring-1 ring-border/60">
            <GitCommitHorizontal className="h-3 w-3 text-muted-foreground/80" />
          </span>
          <h3 className="text-xs font-medium text-muted-foreground">
            Commits on {group.label}
          </h3>
          <div className="relative mt-2">
            {/* Stub linking the timeline rail to the commit box. */}
            <span
              aria-hidden="true"
              className="absolute top-8 -left-[1.3125rem] h-px w-[1.3125rem] bg-border/60"
            />
            <div className="divide-y divide-border/50 overflow-hidden rounded-xl border border-border/60 bg-card">
              {group.commits.map((commit) => {
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
                          authored {relativeCommitTime(commit.timestamp)}
                        </span>
                        {branch ? (
                          <span className="inline-flex min-w-0 items-center gap-1 rounded-full border border-border/60 px-1.5 py-0.5 font-mono text-2xs">
                            <GitBranch className="h-3 w-3 shrink-0" />
                            <span className="truncate">{branch}</span>
                          </span>
                        ) : null}
                        {matchingContributor?.commitCount ? (
                          <span className="rounded-full border border-border/60 px-1.5 py-0.5 text-2xs">
                            {pluralize(
                              matchingContributor.commitCount,
                              "commit",
                            )}
                          </span>
                        ) : null}
                      </>
                    }
                    onOpen={
                      onSelectCommit ? () => onSelectCommit(commit) : undefined
                    }
                    testId="project-activity-feed-item"
                    title={commit.subject}
                    trailing={
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
                    }
                  />
                );
              })}
            </div>
          </div>
        </section>
      ))}
    </div>
  );
}

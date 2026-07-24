import { GitCommitHorizontal } from "lucide-react";

import {
  profileForCommit,
  type ViewerGitIdentity,
} from "@/features/projects/lib/projectContributorMatching";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { ProjectRepoCommit, ProjectRepoDiff } from "@/shared/api/types";
import { CopyCommitHashButton } from "./ProjectCommitCopyButton";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";
import { ProjectDiffFilesPanel } from "./ProjectPullRequestFilesChangedPanel";

function commitDateLabel(timestamp: number) {
  return new Date(timestamp * 1_000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/**
 * Detail view for a single commit: header with author identity and hash,
 * followed by the commit-vs-parent diff rendered with the shared changed
 * files panel.
 */
export function ProjectCommitDetailPanel({
  commit,
  commitAuthorPubkeys,
  commitHash,
  diff,
  diffError,
  diffLoading,
  profiles,
  viewerGitIdentity,
}: {
  commit: ProjectRepoCommit | null;
  /** Signed commit→pubkey mapping derived from pull request events. */
  commitAuthorPubkeys?: Map<string, string>;
  commitHash: string;
  diff: ProjectRepoDiff | null | undefined;
  diffError: unknown;
  diffLoading: boolean;
  profiles?: UserProfileLookup;
  viewerGitIdentity?: ViewerGitIdentity | null;
}) {
  const matchedProfile = commit
    ? profileForCommit(commit, profiles, commitAuthorPubkeys, viewerGitIdentity)
    : null;
  const authorLabel = matchedProfile
    ? resolveUserLabel({ pubkey: matchedProfile.pubkey, profiles })
    : (commit?.authorName ?? commit?.authorEmail ?? "Unknown author");
  const shortHash = commit?.shortHash ?? commitHash.slice(0, 7);

  return (
    <div className="space-y-3">
      <header className="space-y-2 rounded-xl border border-border/60 bg-card p-4">
        <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
          <GitCommitHorizontal className="h-3.5 w-3.5" />
          Commit from {authorLabel}
        </p>
        <div className="flex min-w-0 items-start gap-3">
          <ProfileIdentityButton
            avatarClassName="mt-0.5 shrink-0"
            avatarSize="md"
            avatarUrl={matchedProfile?.profile.avatarUrl ?? null}
            isAgent={matchedProfile?.profile.isAgent === true}
            label={authorLabel}
            pubkey={matchedProfile?.pubkey ?? null}
            showLabel={false}
          />
          <div className="min-w-0 flex-1 space-y-1">
            <h3 className="line-clamp-2 text-base font-semibold text-foreground">
              {commit?.subject ?? shortHash}
            </h3>
            <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-4 text-muted-foreground">
              <span className="flex items-center gap-0.5 font-mono">
                {shortHash}
                <CopyCommitHashButton
                  className="h-6 w-6"
                  hash={commit?.hash ?? commitHash}
                />
              </span>
              {commit ? (
                <>
                  <span>·</span>
                  <span>{commitDateLabel(commit.timestamp)}</span>
                </>
              ) : null}
              {diff ? (
                <>
                  <span>·</span>
                  <span className="text-green-500">+{diff.additions}</span>
                  <span className="text-destructive">-{diff.deletions}</span>
                </>
              ) : null}
            </div>
          </div>
        </div>
      </header>

      <ProjectDiffFilesPanel
        diff={diff}
        error={diffError}
        headerLabel={`${commit?.subject ?? "Commit"} · ${shortHash}`}
        isLoading={diffLoading}
        subjectLabel="commit"
      />
    </div>
  );
}

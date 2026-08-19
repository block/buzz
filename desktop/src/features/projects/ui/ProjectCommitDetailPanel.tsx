import { Calendar, GitCommitHorizontal, Hash } from "lucide-react";

import {
  profileForCommit,
  type ViewerGitIdentity,
} from "@/features/projects/lib/projectContributorMatching";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { Repository } from "@/features/projects/hooks";
import { commitDiscussionQuery } from "@/features/projects/lib/discussionChannels";
import { commitShareLink } from "@/features/projects/lib/projectShareLinks";
import type { ProjectRepoCommit, ProjectRepoDiff } from "@/shared/api/types";
import { DiscussedInChannels } from "./DiscussionChannels";
import { CopyCommitHashButton } from "./ProjectCommitCopyButton";
import {
  ProjectDetailMetaList,
  ProjectDetailMetaRow,
} from "./ProjectDetailMeta";
import { ProjectDetailSection } from "./ProjectDetailSection";
import { PROJECT_DETAIL_READING_COLUMN_CLASS } from "./projectPanelStyles";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";
import { ProjectDiffFilesPanel } from "./ProjectPullRequestFilesChangedPanel";
import { ProjectOriginReference } from "./ProjectOriginReference";
import { ProjectRichContent } from "./ProjectRichContent";
import { ShareLinkButton } from "./ShareLinkButton";

function commitDateLabel(timestamp: number) {
  return new Date(timestamp * 1_000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/**
 * Detail view for a single commit: title and meta header, then collapsible
 * description and the commit-vs-parent diff.
 */
export function ProjectCommitDetailPanel({
  commit,
  commitAuthorPubkeys,
  commitHash,
  diff,
  diffError,
  diffLoading,
  originAgentName,
  originChannelId,
  profiles,
  project,
  viewerGitIdentity,
}: {
  commit: ProjectRepoCommit | null;
  /** Signed commit→pubkey mapping derived from pull request events. */
  commitAuthorPubkeys?: Map<string, string>;
  commitHash: string;
  diff: ProjectRepoDiff | null | undefined;
  diffError: unknown;
  diffLoading: boolean;
  originAgentName?: string | null;
  originChannelId?: string | null;
  profiles?: UserProfileLookup;
  project: Repository;
  viewerGitIdentity?: ViewerGitIdentity | null;
}) {
  const matchedProfile = commit
    ? profileForCommit(commit, profiles, commitAuthorPubkeys, viewerGitIdentity)
    : null;
  const authorLabel = matchedProfile
    ? resolveUserLabel({ pubkey: matchedProfile.pubkey, profiles })
    : (commit?.authorName ?? commit?.authorEmail ?? "Unknown author");
  const shortHash = commit?.shortHash ?? commitHash.slice(0, 7);
  const fileCount = diff?.files.length;

  return (
    <div
      className={PROJECT_DETAIL_READING_COLUMN_CLASS}
      data-project-detail-panel
      data-testid="project-commit-detail"
    >
      <header className="space-y-2 px-6 pb-3 pt-5">
        <h3 className="line-clamp-2 text-lg font-semibold leading-6 text-foreground">
          {commit?.subject ?? shortHash}{" "}
          <ShareLinkButton
            className="ml-1 inline-flex h-7 w-7 align-text-bottom"
            label="Copy commit link"
            link={commitShareLink(project, commit?.hash ?? commitHash)}
            testId="project-commit-copy-link"
          />
        </h3>
        <p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
          <ProfileIdentityButton
            avatarClassName="shrink-0"
            avatarSize="xs"
            avatarUrl={matchedProfile?.profile.avatarUrl ?? null}
            isAgent={matchedProfile?.profile.isAgent === true}
            label={authorLabel}
            pubkey={matchedProfile?.pubkey ?? null}
            showLabel={false}
          />
          <span className="font-medium text-foreground">{authorLabel}</span>
          {commit ? (
            <span
              className="shrink-0 whitespace-nowrap"
              title={commitDateLabel(commit.timestamp)}
            >
              {commitDateLabel(commit.timestamp)}
            </span>
          ) : null}
          <ProjectOriginReference
            agentName={originAgentName}
            channelId={originChannelId}
          />
        </p>
      </header>
      <ProjectDetailMetaList>
        <ProjectDetailMetaRow icon={Hash} label="Commit">
          <span className="inline-flex min-w-0 items-center gap-1 font-mono text-xs">
            {shortHash}
            <CopyCommitHashButton
              className="h-6 w-6"
              hash={commit?.hash ?? commitHash}
            />
          </span>
        </ProjectDetailMetaRow>
        {commit ? (
          <ProjectDetailMetaRow icon={Calendar} label="Date">
            <span className="text-muted-foreground">
              {commitDateLabel(commit.timestamp)}
            </span>
          </ProjectDetailMetaRow>
        ) : null}
        {diff ? (
          <ProjectDetailMetaRow icon={GitCommitHorizontal} label="Changes">
            <span className="flex items-center gap-1.5">
              <span className="text-green-500">+{diff.additions}</span>
              <span className="text-destructive">-{diff.deletions}</span>
            </span>
          </ProjectDetailMetaRow>
        ) : null}
      </ProjectDetailMetaList>
      {diff?.commitBody ? (
        <ProjectDetailSection defaultOpen title="Description">
          <ProjectRichContent
            content={diff.commitBody}
            hardLineBreaks={false}
          />
        </ProjectDetailSection>
      ) : null}
      <DiscussedInChannels
        className="mx-6 mb-4"
        entityLabel="this commit"
        query={commitDiscussionQuery({
          hash: commit?.hash ?? commitHash,
          shortHash: commit?.shortHash,
        })}
        testId="commit-discussed-in"
      />
      <ProjectDetailSection count={fileCount} defaultOpen title="Files changed">
        <div className="-mx-6">
          <ProjectDiffFilesPanel
            diff={diff}
            embedded
            error={diffError}
            headerLabel={`${commit?.subject ?? "Commit"} · ${shortHash}`}
            isLoading={diffLoading}
            subjectLabel="commit"
          />
        </div>
      </ProjectDetailSection>
    </div>
  );
}

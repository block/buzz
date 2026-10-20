import { Calendar, GitCommitHorizontal, Hash, UserRound } from "lucide-react";

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
  commitHash,
  diff,
  diffError,
  diffLoading,
  originAgentName,
  originChannelId,
  project,
}: {
  commit: ProjectRepoCommit | null;
  commitHash: string;
  diff: ProjectRepoDiff | null | undefined;
  diffError: unknown;
  diffLoading: boolean;
  originAgentName?: string | null;
  originChannelId?: string | null;
  project: Repository;
}) {
  const shortHash = commit?.shortHash ?? commitHash.slice(0, 7);

  return (
    <div
      className="flex w-full flex-1 flex-col overflow-hidden"
      data-project-detail-panel
      data-testid="project-commit-detail"
    >
      <div className={PROJECT_DETAIL_READING_COLUMN_CLASS}>
        <header className="space-y-1 px-6 pb-3 pt-5">
          <h3 className="line-clamp-2 text-lg font-semibold leading-6 text-foreground">
            {commit?.subject ?? shortHash}{" "}
            <ShareLinkButton
              className="ml-0.5 inline-flex h-auto w-auto align-middle hover:bg-transparent"
              label="Copy commit link"
              link={commitShareLink(project, commit?.hash ?? commitHash)}
              testId="project-commit-copy-link"
            />
          </h3>
          <p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
            <span>Committed</span>
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
          {commit?.authorName ? (
            <ProjectDetailMetaRow icon={UserRound} label="Author">
              <span
                className="min-w-0 truncate text-muted-foreground"
                title={commit.authorEmail}
              >
                {commit.authorName}
              </span>
            </ProjectDetailMetaRow>
          ) : null}
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
      </div>
      <ProjectDetailSection
        className="flex min-h-0 flex-1 flex-col"
        contentClassName="flex min-h-0 flex-1 flex-col"
        defaultOpen
        headerClassName="mx-auto max-w-3xl"
        title="Files changed"
      >
        {/* Full-bleed: cancel the section's inner padding so the file
            tree + diff grid spans the whole content column, and let it
            grow to the bottom of the scrollport when the page is short. */}
        <div className="-mx-6 -mb-6 flex min-h-0 flex-1 flex-col">
          <ProjectDiffFilesPanel
            className="min-h-0 flex-1"
            fileTreeClassName="max-h-none"
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

import { ArrowLeft, ExternalLink } from "lucide-react";
import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import type { ProjectInboxWorkItem } from "@/features/home/lib/projectInbox";
import { ProjectIssueDetail } from "@/features/projects/ui/ProjectIssuesPanel";
import { ProjectPullRequestDetail } from "@/features/projects/ui/ProjectPullRequestsPanel";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { openProjectMergeRecoveryTerminal } from "@/shared/api/projectGit";
import { useElementWidth } from "@/shared/hooks/use-mobile";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type ProjectInboxDetailPaneProps = {
  isSinglePanelView?: boolean;
  onBack?: () => void;
  onOpenProject: () => void;
  profiles?: UserProfileLookup;
  workItem: ProjectInboxWorkItem;
};

/** Renders a canonical Buzz Git work item with its existing project actions. */
export function ProjectInboxDetailPane({
  isSinglePanelView = false,
  onBack,
  onOpenProject,
  profiles,
  workItem,
}: ProjectInboxDetailPaneProps) {
  const { activeCommunity } = useCommunities();
  const [detailContentRef, detailContentWidth] =
    useElementWidth<HTMLDivElement>();
  const showWideActions = detailContentWidth >= 760;
  const authorPubkey =
    workItem.type === "pull-request"
      ? workItem.pullRequest.author
      : workItem.issue.author;
  const authorLabel = resolveUserLabel({ profiles, pubkey: authorPubkey });
  const authorAvatarUrl =
    profiles?.[normalizePubkey(authorPubkey)]?.avatarUrl ?? null;
  const authorIsAgent =
    profiles?.[normalizePubkey(authorPubkey)]?.isAgent === true;
  const inboxTitle = `${authorLabel} sent you ${
    workItem.type === "pull-request" ? "a review" : "a task"
  }`;
  // The action deep-links to this specific work item in the project view,
  // so the label names the entity, not the project.
  const openLabel =
    workItem.type === "pull-request" ? "Open review" : "Open task";
  const handleOpenMergeRecoveryTerminal = React.useCallback(
    async (input: {
      expectedCommit: string;
      sourceBranch: string;
      sourceCloneUrl: string;
      targetBranch: string;
    }) => {
      if (workItem.type !== "pull-request") {
        throw new Error("Merge recovery is only available for reviews.");
      }
      const targetCloneUrl = workItem.repository.cloneUrls[0];
      if (!targetCloneUrl) {
        throw new Error("This repository has no clone URL.");
      }
      return openProjectMergeRecoveryTerminal({
        ...input,
        projectDtag: workItem.repository.dtag,
        reposDir: activeCommunity?.reposDir,
        targetCloneUrl,
      });
    },
    [activeCommunity?.reposDir, workItem],
  );

  return (
    <section
      className="flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60"
      data-testid="home-project-inbox-detail"
    >
      <TopChromeInsetHeader flush transparent>
        <div className="px-5 py-2">
          <div className="flex min-h-9 min-w-0 items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-1">
              {isSinglePanelView && onBack ? (
                <Button
                  aria-label="Back to Inbox"
                  onClick={onBack}
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <ArrowLeft className="h-4 w-4" />
                </Button>
              ) : null}
              <UserAvatar
                avatarUrl={authorAvatarUrl}
                className="shrink-0"
                displayName={authorLabel}
                shape={authorIsAgent ? "squircle" : "circle"}
                size="sm"
                testId="project-inbox-author-avatar"
              />
              <h2
                className="min-w-0 translate-y-px truncate text-sm font-semibold leading-5 tracking-tight text-foreground"
                title={`${inboxTitle} · ${workItem.project.name}`}
              >
                {inboxTitle}
              </h2>
            </div>
            <Button
              aria-label={openLabel}
              className="shrink-0"
              onClick={onOpenProject}
              size={showWideActions ? "sm" : "icon"}
              title={openLabel}
              type="button"
              variant="ghost"
            >
              <ExternalLink className="h-4 w-4" />
              {showWideActions ? openLabel : null}
            </Button>
          </div>
        </div>
      </TopChromeInsetHeader>

      <div
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain"
        ref={detailContentRef}
      >
        <div className="p-3">
          <div
            className="overflow-hidden rounded-xl border border-border/60 bg-card"
            data-testid="project-inbox-work-item-card"
          >
            <div data-testid="project-inbox-work-item-layout">
              {workItem.type === "pull-request" ? (
                <ProjectPullRequestDetail
                  onOpenTerminal={handleOpenMergeRecoveryTerminal}
                  profiles={profiles}
                  project={workItem.repository}
                  pullRequest={workItem.pullRequest}
                />
              ) : (
                <ProjectIssueDetail
                  issue={workItem.issue}
                  profiles={profiles}
                  project={workItem.repository}
                />
              )}
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

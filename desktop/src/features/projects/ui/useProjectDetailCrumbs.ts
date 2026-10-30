import type * as React from "react";

import type { EntityLinkTab } from "@/shared/lib/entityLink";
import type { ProjectDetailWorkItemCrumb } from "./ProjectDetailChrome";
import { PROJECT_TAB_CRUMB_LABELS } from "./projectDetailHelpers";

export function buildProjectDetailCrumbs({
  activeTab,
  commit,
  issue,
  pullRequest,
  setRequestedTab,
  setSelectedCommitHash,
  setSelectedIssueId,
  setSelectedPullRequestId,
  setTabsResetKey,
}: {
  activeTab: string;
  commit: { hash: string; subject?: string | null } | null;
  issue: { id: string; title: string } | null;
  pullRequest: { id: string; title: string } | null;
  setRequestedTab: (tab: EntityLinkTab | undefined) => void;
  setSelectedCommitHash: (hash: string | null) => void;
  setSelectedIssueId: (id: string | null) => void;
  setSelectedPullRequestId: (id: string | null) => void;
  setTabsResetKey: React.Dispatch<React.SetStateAction<number>>;
}) {
  const activeWorkItemCrumb: ProjectDetailWorkItemCrumb | null = pullRequest
    ? {
        category: "Review",
        clear: () => setSelectedPullRequestId(null),
        title: pullRequest.title,
      }
    : issue
      ? {
          category: "Tasks",
          clear: () => setSelectedIssueId(null),
          title: issue.title,
        }
      : commit
        ? {
            category: "Commits",
            clear: () => setSelectedCommitHash(null),
            title: commit.subject || commit.hash.slice(0, 7),
          }
        : null;
  const activeTabCrumb = activeWorkItemCrumb
    ? null
    : (PROJECT_TAB_CRUMB_LABELS[activeTab] ?? null);
  const handleGoToProjectHome = () => {
    setSelectedPullRequestId(null);
    setSelectedIssueId(null);
    setSelectedCommitHash(null);
    setRequestedTab(undefined);
    setTabsResetKey((key) => key + 1);
  };
  return { activeTabCrumb, activeWorkItemCrumb, handleGoToProjectHome };
}

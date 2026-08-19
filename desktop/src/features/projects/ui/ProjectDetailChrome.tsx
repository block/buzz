import { ChevronRight, FolderGit2 } from "lucide-react";
import type * as React from "react";

import type { Project } from "@/features/projects/hooks";
import { projectShareLink } from "@/features/projects/lib/projectShareLinks";
import { channelChrome, topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import type { EntityLinkTab } from "@/shared/lib/entityLink";
import { ShareLinkButton } from "./ShareLinkButton";

export type ProjectDetailWorkItemCrumb = {
  category: string;
  title: string;
  clear: () => void;
};

export function ProjectDetailChrome({
  actions,
  activeTabCrumb,
  activeWorkItemCrumb,
  chromeRef,
  onGoProjectHome,
  onGoProjects,
  project,
  shareTab,
}: {
  /** Repository-scoped controls, rendered left of the project-wide ones. */
  actions?: React.ReactNode;
  activeTabCrumb: string | null;
  activeWorkItemCrumb: ProjectDetailWorkItemCrumb | null;
  chromeRef: React.Ref<HTMLDivElement>;
  onGoProjectHome: () => void;
  onGoProjects: () => void;
  project: Project;
  /**
   * Workspace tab the copied link should open (`undefined` = overview), so
   * sharing from the PR or issue list lands recipients on that same list.
   */
  shareTab?: EntityLinkTab;
}) {
  return (
    <div
      className={cn(
        "pointer-events-none relative z-30 overflow-hidden rounded-tl-xl bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
        channelChrome.negativeMargin,
        topChromeInset.divider,
      )}
      ref={chromeRef}
    >
      <div
        className="pointer-events-auto flex min-h-[2.75rem] items-center justify-between gap-3 px-4 py-1.5"
        data-tauri-drag-region
      >
        <nav
          aria-label="Project breadcrumb"
          className="-ml-1 flex min-w-0 items-center gap-0.5 text-xs text-muted-foreground"
        >
          <button
            className="flex shrink-0 items-center gap-1.5 rounded-md px-1 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            onClick={onGoProjects}
            type="button"
          >
            <FolderGit2 className="h-3.5 w-3.5" />
            Projects
          </button>
          <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" />
          {activeWorkItemCrumb ? (
            <>
              <button
                className="min-w-0 truncate rounded-md px-0.5 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                onClick={onGoProjectHome}
                type="button"
              >
                {project.name}
              </button>
              <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" />
              <button
                className="shrink-0 rounded-md px-0.5 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                onClick={activeWorkItemCrumb.clear}
                type="button"
              >
                {activeWorkItemCrumb.category}
              </button>
              <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" />
              <span
                aria-current="page"
                className="min-w-0 truncate px-0.5 font-medium text-muted-foreground/60"
              >
                {activeWorkItemCrumb.title}
              </span>
            </>
          ) : activeTabCrumb ? (
            <>
              <button
                className="min-w-0 truncate rounded-md px-0.5 py-1 font-medium transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                onClick={onGoProjectHome}
                type="button"
              >
                {project.name}
              </button>
              <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" />
              <span
                aria-current="page"
                className="min-w-0 truncate px-0.5 font-medium text-muted-foreground/60"
              >
                {activeTabCrumb}
              </span>
            </>
          ) : (
            <span
              aria-current="page"
              className="min-w-0 truncate px-0.5 font-medium text-muted-foreground/60"
            >
              {project.name}
            </span>
          )}
        </nav>
        <div className="flex shrink-0 items-center gap-1.5">
          {actions}
          <ShareLinkButton
            label="Copy project link"
            link={projectShareLink(project, shareTab)}
            testId="project-detail-copy-link"
          />
        </div>
      </div>
    </div>
  );
}

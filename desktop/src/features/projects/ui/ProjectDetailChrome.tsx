import { ChevronRight, Folders } from "lucide-react";
import type * as React from "react";

import { AppTopChromePortal } from "@/app/AppTopChromePortal";
import type { Project, Repository } from "@/features/projects/hooks";
import { projectShareLink } from "@/features/projects/lib/projectShareLinks";
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
  onGoProjectHome,
  onGoProjects,
  project,
  repository,
  shareTab,
}: {
  /** Repository-scoped controls, rendered left of the project-wide ones. */
  actions?: React.ReactNode;
  activeTabCrumb: string | null;
  activeWorkItemCrumb: ProjectDetailWorkItemCrumb | null;
  onGoProjectHome: () => void;
  onGoProjects: () => void;
  project: Project;
  repository: Repository;
  shareTab?: EntityLinkTab;
}) {
  return (
    <AppTopChromePortal>
      <div
        className="flex min-w-0 flex-1 items-center justify-between gap-3 pl-2"
        data-tauri-drag-region
        data-testid="project-detail-chrome"
      >
        <nav
          aria-label="Project breadcrumb"
          className="absolute flex max-w-[50%] min-w-0 -translate-x-1/2 -translate-y-px items-center gap-0.5 text-xs text-sidebar-foreground/65"
          style={{
            left: "calc(50% + var(--app-top-chrome-center-offset, 0rem))",
          }}
        >
          <button
            className="flex shrink-0 items-center gap-1.5 rounded-md px-1 py-1 font-medium transition-colors hover:text-sidebar-accent-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            onClick={onGoProjects}
            type="button"
          >
            <Folders className="h-3.5 w-3.5" />
            Projects
          </button>
          <ChevronRight className="h-3 w-3 shrink-0 opacity-60" />
          <button
            className="min-w-0 truncate rounded-md px-0.5 py-1 font-medium transition-colors hover:text-sidebar-accent-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="project-breadcrumb-project"
            onClick={onGoProjectHome}
            type="button"
          >
            {project.name}
          </button>
          <ChevronRight className="h-3 w-3 shrink-0 opacity-60" />
          {activeWorkItemCrumb ? (
            <>
              <button
                className="min-w-0 truncate rounded-md px-0.5 py-1 font-medium transition-colors hover:text-sidebar-accent-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                data-testid="project-breadcrumb-repository"
                onClick={onGoProjectHome}
                type="button"
              >
                {repository.name}
              </button>
              <ChevronRight className="h-3 w-3 shrink-0 opacity-60" />
              <button
                className="shrink-0 rounded-md px-0.5 py-1 font-medium transition-colors hover:text-sidebar-accent-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                onClick={activeWorkItemCrumb.clear}
                type="button"
              >
                {activeWorkItemCrumb.category}
              </button>
              <ChevronRight className="h-3 w-3 shrink-0 opacity-60" />
              <span
                aria-current="page"
                className="min-w-0 truncate px-0.5 font-medium opacity-60"
              >
                {activeWorkItemCrumb.title}
              </span>
            </>
          ) : activeTabCrumb ? (
            <>
              <button
                className="min-w-0 truncate rounded-md px-0.5 py-1 font-medium transition-colors hover:text-sidebar-accent-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                data-testid="project-breadcrumb-repository"
                onClick={onGoProjectHome}
                type="button"
              >
                {repository.name}
              </button>
              <ChevronRight className="h-3 w-3 shrink-0 opacity-60" />
              <span
                aria-current="page"
                className="min-w-0 truncate px-0.5 font-medium opacity-60"
              >
                {activeTabCrumb}
              </span>
            </>
          ) : (
            <span
              aria-current="page"
              className="min-w-0 truncate px-0.5 font-medium opacity-60"
              data-testid="project-breadcrumb-repository"
            >
              {repository.name}
            </span>
          )}
        </nav>
        <div className="ml-auto flex shrink-0 items-center gap-0.5">
          <ShareLinkButton
            className="text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
            label="Copy project link"
            link={projectShareLink(project, shareTab)}
            testId="project-detail-copy-link"
          />
          {actions}
        </div>
      </div>
    </AppTopChromePortal>
  );
}

const BREADCRUMB_BUTTON_CLASS =
  "flex shrink-0 items-center gap-1.5 rounded-md px-1 py-1 font-medium transition-colors hover:text-sidebar-accent-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring";

export function ProjectsWorkspaceChrome({
  actions,
  onGoActivity,
  section,
}: {
  actions: React.ReactNode;
  onGoActivity: () => void;
  section: string;
}) {
  const onActivity = section === "Activity";

  return (
    <AppTopChromePortal>
      <div
        className="flex min-w-0 flex-1 items-center justify-between gap-3 pl-2"
        data-tauri-drag-region
        data-testid="projects-workspace-chrome"
      >
        <nav
          aria-label="Projects breadcrumb"
          className="absolute flex max-w-[50%] min-w-0 -translate-x-1/2 -translate-y-px items-center gap-0.5 text-xs text-sidebar-foreground/65"
          style={{
            left: "calc(50% + var(--app-top-chrome-center-offset, 0rem))",
          }}
        >
          {onActivity ? (
            <span className="flex min-w-0 items-center gap-1.5 px-1 py-1 font-medium">
              <Folders className="h-3.5 w-3.5 shrink-0" />
              Projects
            </span>
          ) : (
            <button
              className={BREADCRUMB_BUTTON_CLASS}
              onClick={onGoActivity}
              type="button"
            >
              <Folders className="h-3.5 w-3.5 shrink-0" />
              Projects
            </button>
          )}
          <ChevronRight className="h-3 w-3 shrink-0 opacity-60" />
          <span
            aria-current="page"
            className="min-w-0 truncate px-0.5 font-medium opacity-60"
          >
            {section}
          </span>
        </nav>
        <div className="ml-auto flex shrink-0 items-center gap-0.5">
          {actions}
        </div>
      </div>
    </AppTopChromePortal>
  );
}

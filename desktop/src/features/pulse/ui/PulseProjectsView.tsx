import * as React from "react";
import { Activity, Folders } from "lucide-react";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { allowNavigation } from "@/app/navigation/navigationGuard";
import { useProjectsQuery } from "@/features/projects/hooks";
import type { ProjectsFilter } from "@/features/projects/lib/projectsViewHelpers";
import { isExplicitProject } from "@/features/projects/projectModels";
import { ProjectBreadcrumbVisibilityContext } from "@/features/projects/ui/ProjectDetailChrome";
import { ProjectsScreen } from "@/features/projects/ui/ProjectsScreen";
import {
  projectsSectionIcon,
  projectsSectionTitle,
} from "@/features/projects/ui/projectsSectionMeta";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { cn } from "@/shared/lib/cn";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import {
  CLEAR_WORKSPACE_PANELS,
  PULSE_WORKSPACE_KEYS,
} from "../lib/workspaceNavigation";
import { CLEAR_CONVERSATION_PANELS } from "../lib/pulsePanelState";

const sections = [
  "all",
  "projects",
  "repositories",
  "issues",
  "prs",
  "channels",
] as const;
const searchKeys = PULSE_WORKSPACE_KEYS;

/** Keep project navigation visible while category content or a project is open. */
export function PulseProjectsView({ detail }: { detail?: React.ReactNode }) {
  const { values, applyPatch } = useHistorySearchState(searchKeys);
  const query = useProjectsQuery();
  const { goProject } = useAppNavigation();
  const section =
    sections.find((section) => section === values.projectSection) ?? "all";
  const projects = React.useMemo(
    () =>
      (query.data ?? [])
        .filter(isExplicitProject)
        .sort((a, b) => a.name.localeCompare(b.name)),
    [query.data],
  );
  const selectSection = (next: ProjectsFilter) => {
    if (
      !allowNavigation({
        kind: "route",
        href: `/pulse?feed=projects&projectSection=${next}`,
      })
    )
      return;
    applyPatch({
      ...CLEAR_CONVERSATION_PANELS,
      ...CLEAR_WORKSPACE_PANELS,
      projectSection: next,
    });
  };
  const rowClass = (active: boolean) =>
    cn(
      "mb-1 flex w-full items-center gap-3 rounded-xl px-3 py-3 text-left text-sm focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
      active ? "bg-muted/70 font-semibold" : "hover:bg-muted/35",
    );
  return (
    <div
      className="flex min-h-0 min-w-0 flex-1"
      data-testid="pulse-projects-split"
    >
      <nav
        aria-label="Project navigation"
        data-testid="pulse-projects-list"
        className="w-[220px] min-w-0 shrink-0 overflow-y-auto border-r border-border/50 p-2"
      >
        {sections.map((item) => {
          const Icon = item === "all" ? Activity : projectsSectionIcon(item);
          const active = !values.projectId && section === item;
          return (
            <button
              type="button"
              key={item}
              aria-current={active ? "page" : undefined}
              data-testid={`projects-section-${item === "all" ? "activity" : item}`}
              onClick={() => selectSection(item)}
              className={rowClass(active)}
            >
              <span
                aria-hidden="true"
                className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground"
              >
                <Icon className="h-4 w-4" />
              </span>
              {projectsSectionTitle(item)}
            </button>
          );
        })}
        <div aria-hidden="true" className="my-2 border-t border-border/40" />
        {projects.map((project) => (
          <button
            key={project.id}
            type="button"
            aria-label={`Open project ${project.name}`}
            aria-current={values.projectId === project.id ? "page" : undefined}
            data-project-id={project.id}
            onClick={() => void goProject(project.id)}
            className={rowClass(values.projectId === project.id)}
          >
            <span
              aria-hidden="true"
              className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground"
            >
              <Folders className="h-4 w-4" />
            </span>
            <span className="min-w-0 truncate">{project.name}</span>
          </button>
        ))}
        {query.isLoading && (
          <p role="status" className="px-3 py-4 text-xs text-muted-foreground">
            Loading projects…
          </p>
        )}
        {query.isError && (
          <button
            type="button"
            onClick={() => void query.refetch()}
            className="px-3 py-4 text-sm text-muted-foreground"
          >
            Retry loading projects
          </button>
        )}
      </nav>
      <div
        className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
        data-testid="pulse-projects-detail"
      >
        <ProjectBreadcrumbVisibilityContext.Provider value={false}>
          <React.Suspense fallback={<ViewLoadingFallback kind="projects" />}>
            {detail ?? (
              <ProjectsScreen
                section={section}
                onSectionChange={selectSection}
              />
            )}
          </React.Suspense>
        </ProjectBreadcrumbVisibilityContext.Provider>
      </div>
    </div>
  );
}

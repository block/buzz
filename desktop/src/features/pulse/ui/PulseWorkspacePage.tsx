import * as React from "react";
import { useLocation } from "@tanstack/react-router";
import { NavigationTargetContext } from "@/app/navigation/NavigationTargetContext";
import { parseProjectDetailSearch } from "@/features/projects/lib/projectDetailSearch";
import {
  parseWorkflowEditorPane,
  serializeWorkflowEditorPane,
} from "@/features/workflows/ui/workflowEditorPane";
import type { WorkflowEditorRoute } from "@/features/workflows/ui/WorkflowsScreen";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { MainInsetProvider } from "@/shared/layout/MainInsetContext";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import {
  type PulseWorkspacePage as WorkspacePage,
  PULSE_WORKSPACE_KEYS,
  workspaceNavigationTarget,
} from "../lib/workspaceNavigation";

const PulseProjectsView = React.lazy(() =>
  import("./PulseProjectsView").then((m) => ({ default: m.PulseProjectsView })),
);
const ProjectDetailScreen = React.lazy(() =>
  import("@/features/projects/ui/ProjectDetailScreen").then((m) => ({
    default: m.ProjectDetailScreen,
  })),
);
const AgentsScreen = React.lazy(() =>
  import("@/features/agents/ui/AgentsScreen").then((m) => ({
    default: m.AgentsScreen,
  })),
);
const WorkflowsRouteScreen = React.lazy(() =>
  import("@/app/routes/WorkflowsRouteScreen").then((m) => ({
    default: m.WorkflowsRouteScreen,
  })),
);
const SEARCH_KEYS = [
  "layout",
  "conversation",
  "channel",
  ...PULSE_WORKSPACE_KEYS,
] as const;

/** Existing Buzz feature pages, hosted in Pulse's main panel. */
export function PulseWorkspacePage({ page }: { page: WorkspacePage }) {
  const mainRef = React.useRef<HTMLDivElement>(null);
  const { values, applyPatch } = useHistorySearchState(SEARCH_KEYS);
  const state = useLocation({ select: (location) => location.state }) as {
    workflowEditorHasOrigin?: boolean;
    entityNavigationId?: string;
  };
  const resolveTarget = React.useCallback<
    NonNullable<React.ContextType<typeof NavigationTargetContext>>
  >(
    (target) =>
      workspaceNavigationTarget(target, values.layout, values.conversation),
    [values.layout, values.conversation],
  );
  const pane = parseWorkflowEditorPane(values.pane);
  const hasOrigin = state.workflowEditorHasOrigin === true;
  const editor: WorkflowEditorRoute | null = values.workflowId
    ? {
        workflowId: values.workflowId,
        mode:
          values.view === "duplicate"
            ? "duplicate"
            : values.view === "edit"
              ? "edit"
              : "detail",
        pane,
        hasOrigin,
      }
    : values.view === "create"
      ? {
          mode: "create",
          initialChannelId: values.channel ?? undefined,
          pane,
          hasOrigin,
        }
      : null;
  return (
    <NavigationTargetContext.Provider value={resolveTarget}>
      <div
        className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
        data-testid={`pulse-workspace-${page}`}
        ref={mainRef}
      >
        <MainInsetProvider mainInsetRef={mainRef}>
          <React.Suspense fallback={<ViewLoadingFallback kind={page} />}>
            {page === "projects" ? (
              <PulseProjectsView
                detail={
                  values.projectId ? (
                    <ProjectDetailScreen
                      {...parseProjectDetailSearch(values)}
                      initialPanelCollapsed
                      projectId={values.projectId}
                      entityNavigationId={state.entityNavigationId}
                    />
                  ) : undefined
                }
              />
            ) : page === "agents" ? (
              <AgentsScreen />
            ) : (
              <WorkflowsRouteScreen
                editor={editor}
                onEditorPaneChange={(next) =>
                  applyPatch(
                    { pane: serializeWorkflowEditorPane(next) ?? null },
                    { replace: true },
                  )
                }
              />
            )}
          </React.Suspense>
        </MainInsetProvider>
      </div>
    </NavigationTargetContext.Provider>
  );
}

import * as React from "react";
import { createFileRoute, useLocation } from "@tanstack/react-router";

import {
  parseWorkflowEditorPane,
  serializeWorkflowEditorPane,
} from "@/features/workflows/ui/workflowEditorPane";
import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { LazyWorkflowsRouteScreen } from "./lazyWorkflowsRouteScreen";

export const Route = createFileRoute("/workflows")({
  component: WorkflowsRouteComponent,
  validateSearch: (search: Record<string, unknown>) => ({
    pane: serializeWorkflowEditorPane(parseWorkflowEditorPane(search.pane)),
    view: search.view === "create" ? search.view : undefined,
  }),
});

function WorkflowsRouteComponent() {
  usePreviewFeatureWarning("workflows");
  const navigate = Route.useNavigate();
  const location = useLocation();
  const { pane, view } = Route.useSearch();
  const hasOrigin =
    (location.state as { workflowEditorHasOrigin?: unknown } | undefined)
      ?.workflowEditorHasOrigin === true;

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="workflows" />}>
      <LazyWorkflowsRouteScreen
        editor={
          view === "create"
            ? {
                hasOrigin,
                mode: "create",
                pane: parseWorkflowEditorPane(pane),
              }
            : null
        }
        onEditorPaneChange={(nextPane) => {
          void navigate({
            replace: true,
            resetScroll: false,
            search: {
              pane: serializeWorkflowEditorPane(nextPane),
              view,
            },
          });
        }}
        selectedWorkflowId={null}
      />
    </React.Suspense>
  );
}

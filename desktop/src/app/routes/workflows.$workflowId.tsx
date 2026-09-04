import * as React from "react";
import { createFileRoute, useLocation } from "@tanstack/react-router";

import {
  parseWorkflowEditorPane,
  serializeWorkflowEditorPane,
} from "@/features/workflows/ui/workflowEditorPane";
import { usePreviewFeatureWarning } from "@/shared/features";
import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";
import { LazyWorkflowsRouteScreen } from "./lazyWorkflowsRouteScreen";

export const Route = createFileRoute("/workflows/$workflowId")({
  component: WorkflowRouteComponent,
  validateSearch: (search: Record<string, unknown>) => ({
    pane: serializeWorkflowEditorPane(parseWorkflowEditorPane(search.pane)),
    view:
      search.view === "edit" || search.view === "duplicate"
        ? search.view
        : undefined,
  }),
});

function WorkflowRouteComponent() {
  usePreviewFeatureWarning("workflows");
  const navigate = Route.useNavigate();
  const location = useLocation();
  const { workflowId } = Route.useParams();
  const { pane, view } = Route.useSearch();
  const hasOrigin =
    (location.state as { workflowEditorHasOrigin?: unknown } | undefined)
      ?.workflowEditorHasOrigin === true;
  const editor: import("@/features/workflows/ui/WorkflowsScreen").WorkflowEditorRoute =
    {
      hasOrigin,
      mode:
        view === "duplicate"
          ? "duplicate"
          : view === "edit"
            ? "edit"
            : "detail",
      pane: parseWorkflowEditorPane(pane),
      workflowId,
    };

  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="workflows" />}>
      <LazyWorkflowsRouteScreen
        editor={editor}
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
      />
    </React.Suspense>
  );
}

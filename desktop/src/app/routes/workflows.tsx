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
    channel: typeof search.channel === "string" ? search.channel : undefined,
    pane: serializeWorkflowEditorPane(parseWorkflowEditorPane(search.pane)),
    view: search.view === "create" ? search.view : undefined,
  }),
});

function WorkflowsRouteComponent() {
  usePreviewFeatureWarning("workflows");
  const navigate = Route.useNavigate();
  const location = useLocation();
  const { channel, pane, view } = Route.useSearch();
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
                initialChannelId: channel,
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
              channel,
              pane: serializeWorkflowEditorPane(nextPane),
              view,
            },
          });
        }}
      />
    </React.Suspense>
  );
}

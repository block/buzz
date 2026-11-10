import * as React from "react";

export const LazyWorkflowsRouteScreen = React.lazy(async () => {
  const module = await import("./WorkflowsRouteScreen");
  return { default: module.WorkflowsRouteScreen };
});

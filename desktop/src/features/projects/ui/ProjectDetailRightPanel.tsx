import type * as React from "react";

import type { ProjectDetailAgentContext } from "@/features/projects/lib/projectDetailAgentContext";
import { ProjectAgentChatPanel } from "./ProjectAgentChatPanel";
import { ProjectRepositoryActionsPanel } from "./ProjectRepositoryActionsPanel";
import type { ProjectRightPanelMode } from "./ProjectRightPanelControls";

type RepositoryPanelProps = React.ComponentProps<
  typeof ProjectRepositoryActionsPanel
>;

export function ProjectDetailRightPanel({
  context,
  detachedRepository = false,
  mode,
  sharedHeaderBackdrop,
  ...repositoryProps
}: RepositoryPanelProps & {
  context: ProjectDetailAgentContext;
  detachedRepository?: boolean;
  mode: ProjectRightPanelMode;
  sharedHeaderBackdrop?: boolean;
}) {
  if (mode === "chat") {
    return (
      <ProjectAgentChatPanel
        canResetWidth={repositoryProps.canResetWidth}
        context={context}
        key={context.repoAddress}
        onResetWidth={repositoryProps.onResetWidth}
        onResizeStart={repositoryProps.onResizeStart}
        sharedHeaderBackdrop={sharedHeaderBackdrop}
        widthPx={repositoryProps.widthPx}
      />
    );
  }
  return (
    <ProjectRepositoryActionsPanel
      detached={detachedRepository}
      {...repositoryProps}
    />
  );
}

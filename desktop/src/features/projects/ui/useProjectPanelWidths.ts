import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import type { ProjectRightPanelMode } from "./ProjectRightPanelControls";

/** Default width of the project context rail (repository details and overview). */
export const PROJECT_CONTEXT_PANEL_DEFAULT_WIDTH_PX = 280;
const PROJECT_CONTEXT_PANEL_MIN_WIDTH_PX = 240;

export function useProjectPanelWidths(mode: ProjectRightPanelMode) {
  const threadPanelWidth = useThreadPanelWidth();
  const repositoryContextWidth = useThreadPanelWidth(undefined, {
    defaultWidthPx: PROJECT_CONTEXT_PANEL_DEFAULT_WIDTH_PX,
    minWidthPx: PROJECT_CONTEXT_PANEL_MIN_WIDTH_PX,
    sessionKey: "buzz.desktop.project-context-width",
  });
  return {
    activeRightPanelWidth:
      mode === "repository" ? repositoryContextWidth : threadPanelWidth,
    threadPanelWidth,
  };
}

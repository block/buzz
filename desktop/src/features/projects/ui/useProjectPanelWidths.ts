import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import type { ProjectRightPanelMode } from "./ProjectRightPanelControls";

export function useProjectPanelWidths(mode: ProjectRightPanelMode) {
  const threadPanelWidth = useThreadPanelWidth();
  const repositoryContextWidth = useThreadPanelWidth(undefined, {
    defaultWidthPx: 280,
    minWidthPx: 240,
    sessionKey: "buzz.desktop.project-context-width",
  });
  return {
    activeRightPanelWidth:
      mode === "repository" ? repositoryContextWidth : threadPanelWidth,
    threadPanelWidth,
  };
}

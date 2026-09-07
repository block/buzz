import {
  DockviewReact,
  type DockviewReadyEvent,
  type IDockviewPanelProps,
  themeLight,
} from "dockview-react";
import { createContext, type ReactNode, useCallback, useContext } from "react";
import "dockview-react/dist/styles/dockview.css";

type WorkspacePanels = {
  navigator: ReactNode;
  conversation: ReactNode;
};

const DEFAULT_NAVIGATOR_WIDTH = 216;
const MIN_NAVIGATOR_WIDTH = 176;

const WorkspacePanelContext = createContext<WorkspacePanels | null>(null);

function WorkspacePanel({ id }: { id: keyof WorkspacePanels }) {
  const panels = useContext(WorkspacePanelContext);
  if (!panels) throw new Error("Workspace panel rendered outside its owner.");
  return panels[id];
}

const components = {
  navigator: (_props: IDockviewPanelProps) => <WorkspacePanel id="navigator" />,
  conversation: (_props: IDockviewPanelProps) => (
    <WorkspacePanel id="conversation" />
  ),
};

/** Layout-only workspace shell. Product panels own their own surface treatment. */
export function DockWorkspace({ panels }: { panels: WorkspacePanels }) {
  const onReady = useCallback((event: DockviewReadyEvent) => {
    if (event.api.panels.length > 0) return;
    const navigator = event.api.addPanel({
      id: "navigator",
      component: "navigator",
      title: "Browse",
      initialWidth: DEFAULT_NAVIGATOR_WIDTH,
      minimumWidth: MIN_NAVIGATOR_WIDTH,
    });
    navigator.group.header.hidden = true;
    const conversation = event.api.addPanel({
      id: "conversation",
      component: "conversation",
      title: "Workspace",
      position: { referencePanel: navigator, direction: "right" },
    });
    conversation.group.header.hidden = true;
    requestAnimationFrame(() => {
      navigator.group.api.setSize({ width: DEFAULT_NAVIGATOR_WIDTH });
    });
  }, []);

  return (
    <WorkspacePanelContext.Provider value={panels}>
      <DockviewReact
        className="buzz-dockview"
        components={components}
        disableFloatingGroups
        dndStrategy="pointer"
        onReady={onReady}
        theme={themeLight}
      />
    </WorkspacePanelContext.Provider>
  );
}

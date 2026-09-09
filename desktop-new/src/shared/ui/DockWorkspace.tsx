import {
  type DockviewApi,
  DockviewReact,
  type DockviewReadyEvent,
  type IDockviewPanelProps,
} from "dockview-react";
import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useColorScheme } from "@/shared/theme/useColorScheme";
import { dockTheme, readPanelGap } from "./dockTheme";

// Dockview's stylesheet is deliberately *not* imported here. It is imported by
// `shared/styles/globals.css` into `@layer vendor`, so everything we author wins
// over it. An unlayered import from this file would beat our own rules on every
// selector regardless of specificity — see the note in `globals.css`.

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
  const { scheme } = useColorScheme();
  // The gutter is a token, and dockview needs it in pixels. Read during the
  // initialiser: starting at 0 lays the grid out once with no gutter, and
  // whether the correction lands depends on a re-measure arriving. The
  // stylesheet is available synchronously, so there is nothing to wait for.
  const [gap, setGap] = useState(() => readPanelGap(document.documentElement));
  const [dockApi, setDockApi] = useState<DockviewApi>();
  const root = useRef<HTMLDivElement>(null);

  // The observer is not belt-and-braces: the gutter is authored in rem, and
  // keyboard zoom scales the root font size, so its pixel value genuinely
  // changes at runtime. The value is mode-independent, so nothing here watches
  // the colour scheme.
  useEffect(() => {
    const documentRoot = document.documentElement;
    const observer = new ResizeObserver(() =>
      setGap(readPanelGap(documentRoot)),
    );
    observer.observe(documentRoot);
    return () => observer.disconnect();
  }, []);

  const theme = useMemo(() => dockTheme(scheme, gap), [scheme, gap]);

  // Marks the dock while a panel is in flight, so the stylesheet can drop the
  // panels' blur for the duration — see `product.css` § Dragging a panel. The
  // flag is an attribute written directly rather than React state: it changes
  // on a pointer gesture, no React subtree depends on it, and re-rendering the
  // dock mid-drag is the cost this exists to avoid.
  //
  // The end of a drag is a pointer/DnD event rather than a dockview event, so
  // every way a gesture can finish clears the flag — dropped, released outside
  // any target, or cancelled by the OS. A missed one would leave the panels
  // flat until the next drag, so the recovery path is the whole point.
  useEffect(() => {
    const api = dockApi;
    const element = root.current;
    if (!api || !element) return;

    const endEvents = [
      "pointerup",
      "pointercancel",
      "dragend",
      "drop",
    ] as const;
    const clear = () => {
      delete element.dataset.dragging;
      for (const name of endEvents) window.removeEventListener(name, clear);
    };
    const begin = () => {
      element.dataset.dragging = "true";
      for (const name of endEvents) window.addEventListener(name, clear);
    };

    const subscriptions = [
      api.onWillDragPanel(begin),
      api.onWillDragGroup(begin),
    ];
    return () => {
      clear();
      for (const subscription of subscriptions) subscription.dispose();
    };
  }, [dockApi]);

  const onReady = useCallback((event: DockviewReadyEvent) => {
    setDockApi(event.api);
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
        ref={root}
        className="buzz-dockview"
        components={components}
        disableFloatingGroups
        dndStrategy="pointer"
        onReady={onReady}
        theme={theme}
      />
    </WorkspacePanelContext.Provider>
  );
}

import {
  DockviewReact,
  type DockviewReadyEvent,
  type IDockviewHeaderActionsProps,
  type IDockviewPanelHeaderProps,
  type IDockviewPanelProps,
} from "dockview-react";
import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useColorScheme } from "@/shared/theme/useColorScheme";
import { dockTheme, readPanelGap } from "./dockTheme";
import { PanelHeader } from "./PanelHeader";
import { canPlaceBentoPanel } from "./bentoPlacement";

export type BentoPanel = {
  id: string;
  title: string;
  /** Feature-owned controls for this panel's header. */
  actions?: ReactNode;
  content?: ReactNode;
  minimumWidth?: number;
  minimumHeight?: number;
};

const Panels = createContext<readonly BentoPanel[]>([]);

function usePanel(id: string | undefined) {
  const panels = useContext(Panels);
  return panels.find((panel) => panel.id === id);
}

/**
 * The panel body. Dockview owns the box; the panel's own surface comes from the
 * group, so this renders content only — a second surface here would be the
 * redundant-fill mistake DESIGN.md § Surface and depth documents.
 */
function Content({ api }: IDockviewPanelProps) {
  return usePanel(api.id)?.content ?? null;
}

/**
 * A tab. Only rendered when a group holds more than one panel: the tab strip is
 * how you *choose* a panel, so a lone panel has nothing to choose between and
 * shows its header alone.
 */
function Tab({ api, containerApi: _containerApi }: IDockviewPanelHeaderProps) {
  const panel = usePanel(api.id);
  return (
    <span className="bento-tab-label text-body text-secondary">
      {panel?.title}
    </span>
  );
}

/**
 * The header, rendered into dockview's right-actions slot so it spans the full
 * header row beneath any tabs.
 *
 * This is the real `PanelHeader`, not a lookalike: dockview supplies the drag
 * behaviour by owning the element around it, and the design system supplies the
 * appearance. That is the whole reason this composition is worth having — the
 * alternative was reproducing header styling inside a vendor tab.
 */
function Header({ activePanel }: IDockviewHeaderActionsProps) {
  const panel = usePanel(activePanel?.id);
  if (!panel) return null;
  return <PanelHeader title={panel.title} actions={panel.actions} />;
}

const components = { content: Content };

/**
 * An arrangement of panels a person can rearrange.
 *
 * Each panel's header is its drag handle. Edge drops split; centre drops cancel
 * while we tune direct manipulation without stacking. Resizing happens in the gutter
 * between panels. Floating is deliberately not a drag outcome — a drag always
 * means "put this somewhere in the grid", so a slightly-off gesture can never
 * fling a panel out of the layout.
 */
export function BentoWorkspace({
  panels,
  anchoredPanelId,
}: {
  panels: readonly BentoPanel[];
  anchoredPanelId?: string;
}) {
  const { scheme } = useColorScheme();
  const cleanup = useRef<(() => void) | undefined>(undefined);
  useEffect(() => () => cleanup.current?.(), []);

  // The gutter between panels is the theme's `gap`, which dockview applies as a
  // splitview margin — real space, with the resize seam inside it. A CSS
  // variable cannot do this: the grid computes panel sizes in JavaScript, so a
  // margin it does not know about just makes the panels touch and shows the
  // vendor's 1px separator instead of air.
  //
  // Read during the initialiser rather than in an Effect. Starting at 0 and
  // correcting afterwards means the grid lays out once with no gutter, and
  // whether the corrected value ever lands depends on the re-measure arriving —
  // which it did in a browser and did not under test, so the panels touched.
  // The stylesheet is available synchronously here; there is nothing to wait for.
  const [gap, setGap] = useState(() => readPanelGap(document.documentElement));

  // The gutter is authored in rem and keyboard zoom scales the root font size,
  // so its pixel value genuinely changes at runtime.
  useEffect(() => {
    const documentRoot = document.documentElement;
    const observer = new ResizeObserver(() =>
      setGap(readPanelGap(documentRoot)),
    );
    observer.observe(documentRoot);
    return () => observer.disconnect();
  }, []);

  const theme = useMemo(
    () => ({
      ...dockTheme(scheme, gap),
      className: `dockview-theme-${scheme} bento-dock-theme`,
    }),
    [scheme, gap],
  );

  function onReady({ api }: DockviewReadyEvent) {
    cleanup.current?.();
    const overlay = api.onWillShowOverlay((event) => {
      if (
        !canPlaceBentoPanel(
          api,
          event,
          anchoredPanelId,
          readPanelGap(document.documentElement),
        )
      )
        event.preventDefault();
    });
    const drop = api.onWillDrop((event) => {
      if (
        !canPlaceBentoPanel(
          api,
          event,
          anchoredPanelId,
          readPanelGap(document.documentElement),
        )
      )
        event.preventDefault();
    });
    const dragGroup = api.onWillDragGroup((event) => {
      if (event.group.panels.some((panel) => panel.id === anchoredPanelId))
        event.nativeEvent.preventDefault();
    });
    const dragPanel = api.onWillDragPanel((event) => {
      if (event.panel.id === anchoredPanelId)
        event.nativeEvent.preventDefault();
    });
    cleanup.current = () => {
      dragGroup.dispose();
      dragPanel.dispose();
      overlay.dispose();
      drop.dispose();
    };
    let previous: string | undefined;
    const ordered = [...panels].sort(
      (a, b) =>
        Number(b.id === anchoredPanelId) - Number(a.id === anchoredPanelId),
    );
    for (const panel of ordered) {
      api.addPanel({
        id: panel.id,
        title: panel.title,
        component: "content",
        initialWidth: panel.id === anchoredPanelId ? 216 : undefined,
        minimumWidth: panel.minimumWidth ?? 180,
        minimumHeight: panel.minimumHeight ?? 120,
        position: previous
          ? {
              referencePanel: previous,
              direction:
                anchoredPanelId && previous !== anchoredPanelId
                  ? "below"
                  : api.width < 600
                    ? "below"
                    : "right",
            }
          : undefined,
      });
      previous = panel.id;
    }
    const anchor = anchoredPanelId ? api.getPanel(anchoredPanelId) : undefined;
    if (anchor) {
      anchor.group.element.dataset.bentoAnchored = "true";
      if (api.width >= 600) anchor.group.api.setSize({ width: 216 });
    }
  }

  return (
    <Panels.Provider value={panels}>
      <div className="bento-layout">
        <DockviewReact
          className="bento-workspace"
          components={components}
          defaultTabComponent={Tab}
          rightHeaderActionsComponent={Header}
          onReady={onReady}
          theme={theme}
          dndStrategy="pointer"
          disableFloatingGroups
        />
      </div>
    </Panels.Provider>
  );
}

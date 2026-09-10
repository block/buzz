import type {
  DockviewApi,
  DockviewWillDropEvent,
  DockviewWillShowOverlayLocationEvent,
} from "dockview-react";

type PlacementEvent =
  | DockviewWillDropEvent
  | DockviewWillShowOverlayLocationEvent;

/** Same policy for the preview and commit; a highlighted destination must be allowed. */
export function canPlaceBentoPanel(
  api: DockviewApi,
  event: PlacementEvent,
  anchoredPanelId?: string,
  gap = 0,
): boolean {
  const data = event.getData();
  const target = event.group;
  const source = api.groups.find((group) => group.id === data?.groupId);
  // Only local, panel-relative splits. Outer-root targets could put a panel
  // above/left of the navigation anchor, and centre drops would create tabs.
  if (
    !data ||
    data.viewId !== api.id ||
    !source ||
    !target ||
    source === target ||
    event.kind !== "content" ||
    event.position === "center"
  )
    return false;
  if (source.panels.some((panel) => panel.id === anchoredPanelId)) return false;
  if (
    target.panels.some((panel) => panel.id === anchoredPanelId) &&
    (event.position === "top" || event.position === "left")
  )
    return false;

  // Dockview previews an even split. Do not invite a split that would need to
  // borrow room from unrelated neighbors to keep either new half usable.
  const horizontal = event.position === "left" || event.position === "right";
  const available = (horizontal ? target.api.width : target.api.height) - gap;
  const minimum = horizontal
    ? Math.max(source.minimumWidth, target.minimumWidth)
    : Math.max(source.minimumHeight, target.minimumHeight);
  const crossAxisFits = horizontal
    ? target.api.height >= Math.max(source.minimumHeight, target.minimumHeight)
    : target.api.width >= Math.max(source.minimumWidth, target.minimumWidth);
  return crossAxisFits && available / 2 >= minimum;
}

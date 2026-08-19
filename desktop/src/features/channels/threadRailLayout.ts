export const THREAD_RAIL_DESKTOP_BREAKPOINT_PX = 600;

export function threadRailShellClassName(_collapsed: boolean): string {
  return [
    "hidden mb-2 mr-2 mt-px max-h-[calc(100%-0.5625rem)] min-h-0 shrink-0 self-start overflow-hidden rounded-2xl bg-background shadow-content-edge ring-1 ring-border/30 ring-inset",
    "min-[601px]:flex min-[601px]:flex-col",
  ].join(" ");
}

export function threadRailHeaderClassName(): string {
  return "flex min-h-13 shrink-0 items-center px-3";
}

export function threadRailEntryClassName(active: boolean): string {
  return [
    "group flex items-center rounded-lg transition-colors hover:bg-muted/70 focus-within:ring-2 focus-within:ring-ring",
    active ? "bg-muted text-foreground" : "",
  ]
    .filter(Boolean)
    .join(" ");
}

type ThreadRailLayoutStore = {
  collapsed: boolean;
  pins: readonly unknown[];
};

export function isThreadRailVisible(
  viewportWidth: number,
  pinCount: number,
): boolean {
  return viewportWidth > THREAD_RAIL_DESKTOP_BREAKPOINT_PX && pinCount > 0;
}

export function projectThreadRailLayout(
  store: ThreadRailLayoutStore,
  viewportWidth: number,
) {
  const pinCount = store.pins.length;
  const collapsed = store.collapsed;

  return {
    visible: isThreadRailVisible(viewportWidth, pinCount),
    pinCount,
    collapsed,
    collapseControl: {
      expanded: !collapsed,
      label: collapsed
        ? `Expand ${pinCount} pinned threads`
        : "Collapse pinned threads",
    },
  };
}

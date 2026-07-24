const SIDEBAR_BACKGROUND_ATTRIBUTE = "data-sidebar-background";

/** Whether a sidebar click landed directly on an opted-in blank surface. */
export function isSidebarBackgroundTarget(target: EventTarget | null): boolean {
  const element =
    typeof Element !== "undefined" && target instanceof Element
      ? target
      : typeof Node !== "undefined" && target instanceof Node
        ? target.parentElement
        : null;
  return element?.hasAttribute(SIDEBAR_BACKGROUND_ATTRIBUTE) ?? false;
}

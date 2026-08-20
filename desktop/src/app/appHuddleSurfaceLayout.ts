export function appHuddleSurfaceClassName(isRoom: boolean): string {
  return [
    "buzz-huddle-app-surface z-10 flex min-h-0 flex-row overflow-hidden",
    isRoom ? "bg-background" : "bg-sidebar",
  ].join(" ");
}

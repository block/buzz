import {
  allowNavigation,
  type GuardedNavigation,
} from "@/app/navigation/navigationGuard";

/**
 * commitGuardedNavigation runs the shared commit flow for app navigations:
 * skip same-destination no-ops, consult the navigation guard, then navigate.
 * `force` and `hasStateUpdate` both defeat the no-op skip — a same-href
 * navigation that writes router state (setting or clearing the search
 * highlight) must commit, or the state never lands. Returns whether the
 * navigation was performed. `deps` exists for unit tests.
 */
export async function commitGuardedNavigation(
  input: {
    currentHref: string;
    nextHref: string;
    force?: boolean;
    guardedTarget: GuardedNavigation;
    hasStateUpdate?: boolean;
    navigate: () => Promise<unknown>;
  },
  deps: {
    allow?: typeof allowNavigation;
  } = {},
): Promise<boolean> {
  const allow = deps.allow ?? allowNavigation;
  if (
    input.currentHref === input.nextHref &&
    !input.force &&
    !input.hasStateUpdate
  ) {
    return false;
  }
  if (!allow(input.guardedTarget)) {
    return false;
  }
  await input.navigate();
  return true;
}

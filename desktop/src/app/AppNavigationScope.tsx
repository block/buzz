import * as React from "react";
import { useLocation } from "@tanstack/react-router";
import {
  NavigationTargetContext,
  type AppNavigationTarget,
} from "./navigation/NavigationTargetContext";
import { workspaceNavigationTarget } from "@/features/pulse/lib/workspaceNavigation";

/** Keep shell-owned dialogs and nested feature navigation in the active workspace. */
export function AppNavigationScope({
  children,
}: {
  children: React.ReactNode;
}) {
  const location = useLocation();
  const inherited = React.useContext(NavigationTargetContext);
  const search = location.search as { conversation?: string };
  const conversation = search.conversation ?? null;
  const resolveTarget = React.useCallback(
    (target: AppNavigationTarget) =>
      workspaceNavigationTarget(target, conversation),
    [conversation],
  );
  return (
    <NavigationTargetContext.Provider
      value={location.pathname === "/pulse" ? resolveTarget : inherited}
    >
      {children}
    </NavigationTargetContext.Provider>
  );
}

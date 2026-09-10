import { createContext } from "react";

/** A canonical app destination before it is committed to router history. */
export type AppNavigationTarget = {
  to: string;
  params?: Record<string, string>;
  search?: Record<string, string | undefined>;
  state?:
    | Record<string, unknown>
    | ((previousState: Record<string, unknown>) => Record<string, unknown>);
};

/** Embedded workspaces can keep canonical feature navigation in their own panel. */
export const NavigationTargetContext = createContext<
  ((target: AppNavigationTarget) => AppNavigationTarget) | null
>(null);

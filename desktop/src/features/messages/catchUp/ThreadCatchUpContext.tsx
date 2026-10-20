import * as React from "react";

import type { ThreadCatchUpController } from "./useThreadCatchUp";

const ThreadCatchUpContext = React.createContext<ThreadCatchUpController>(null);

/**
 * Hands the channel-owned recap controller to the thread panel. Surfaces
 * without a provider (project conversations, huddles) render no recap.
 */
export function ThreadCatchUpProvider({
  children,
  value,
}: {
  children: React.ReactNode;
  value: ThreadCatchUpController;
}) {
  return (
    <ThreadCatchUpContext.Provider value={value}>
      {children}
    </ThreadCatchUpContext.Provider>
  );
}

/** The controller for `threadHeadId`, or null when it serves another thread. */
export function useThreadCatchUp(threadHeadId: string | null | undefined) {
  const controller = React.useContext(ThreadCatchUpContext);
  if (!controller || !threadHeadId) return null;
  return controller.scope.rootEventId === threadHeadId.toLowerCase()
    ? controller
    : null;
}

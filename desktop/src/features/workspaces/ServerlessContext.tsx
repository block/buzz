import { createContext, useContext } from "react";
import type { ReactNode } from "react";

/**
 * Whether the active workspace runs in serverless mode (generic public relay,
 * no Sprout server). Components read this to hide server-only surfaces:
 * search, presence fan-out, huddles, pulse, workflows, and git hosting.
 *
 * Channels, DMs, messages, and agents work in both modes, so they are not
 * gated. See docs/SPROUT_LITE_MODE.md.
 */
const ServerlessContext = createContext<boolean>(false);

export function ServerlessProvider({
  serverless,
  children,
}: {
  serverless: boolean;
  children: ReactNode;
}) {
  return (
    <ServerlessContext.Provider value={serverless}>
      {children}
    </ServerlessContext.Provider>
  );
}

/** Read the active workspace's serverless flag. Defaults to false. */
export function useIsServerless(): boolean {
  return useContext(ServerlessContext);
}

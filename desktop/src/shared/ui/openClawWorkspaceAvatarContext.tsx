import * as React from "react";

import { normalizePubkey } from "@/shared/lib/pubkey";

const EMPTY_OPENCLAW_WORKSPACE_PUBKEYS: ReadonlySet<string> = new Set();

/**
 * Pubkeys of *local* managed agents with `useOpenClawWorkspace` enabled.
 * Published once by `KnownAgentPubkeysProvider` so avatar surfaces can overlay
 * the crab badge without per-row managed-agent query observers.
 */
const OpenClawWorkspaceAvatarContext = React.createContext<ReadonlySet<string>>(
  EMPTY_OPENCLAW_WORKSPACE_PUBKEYS,
);

export function OpenClawWorkspaceAvatarProvider({
  children,
  pubkeys,
}: {
  children: React.ReactNode;
  pubkeys: ReadonlySet<string>;
}) {
  return (
    <OpenClawWorkspaceAvatarContext.Provider value={pubkeys}>
      {children}
    </OpenClawWorkspaceAvatarContext.Provider>
  );
}

/** True when `pubkey` is a local agent with OpenClaw workspace opted in. */
export function useOpenClawWorkspaceAvatarEnabled(
  pubkey?: string | null,
): boolean {
  const pubkeys = React.useContext(OpenClawWorkspaceAvatarContext);
  if (!pubkey) return false;
  const key = normalizePubkey(pubkey);
  return Boolean(key) && pubkeys.has(key);
}

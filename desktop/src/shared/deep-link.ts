import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { toast } from "sonner";

import type { Workspace } from "@/features/workspaces/types";
import {
  deriveWorkspaceName,
  normalizeRelayUrl,
} from "@/features/workspaces/workspaceStorage";

export interface DeepLinkDeps {
  addWorkspace: (workspace: Workspace) => string;
  switchWorkspace: (id: string) => void;
  reconnectWorkspace: () => void;
}

/**
 * Payload emitted by the Rust deep-link handler for `sprout://message?…`.
 * Field names match the JSON shape produced in `desktop/src-tauri/src/lib.rs`.
 */
export type MessageDeepLinkPayload = {
  channelId: string;
  messageId: string;
  threadRootId: string | null;
};

/**
 * Register listeners for deep-link events emitted by the Rust backend.
 *
 * When a `sprout://connect?relay=<url>` link is opened, the handler
 * adds a workspace for the relay (deduplicating by URL) and switches
 * to it. Returns an unlisten function to tear down all listeners.
 *
 * `sprout://message?…` is handled separately by `listenForMessageDeepLinks`,
 * because it needs to dispatch into the router which only exists below the
 * `RouterProvider` in the component tree.
 */
export function listenForDeepLinks(deps: DeepLinkDeps): Promise<UnlistenFn> {
  return listen<string>("deep-link-connect", (event) => {
    const relayUrl = normalizeRelayUrl(event.payload);
    const name = deriveWorkspaceName(relayUrl);
    const id = deps.addWorkspace({
      id: crypto.randomUUID(),
      name,
      relayUrl,
      addedAt: new Date().toISOString(),
    });
    deps.switchWorkspace(id);
    // If addWorkspace returned the already-active workspace (same relay URL),
    // switchWorkspace is a no-op — force re-init so the connection refreshes.
    deps.reconnectWorkspace();
    toast.success(`Connected to ${name}`);
  });
}

/**
 * Register a listener for `deep-link-message` events. Must be called from
 * inside the router tree (e.g. AppShell) because the navigation callback
 * uses TanStack Router state.
 */
export function listenForMessageDeepLinks(
  onOpen: (payload: MessageDeepLinkPayload) => void,
): Promise<UnlistenFn> {
  return listen<MessageDeepLinkPayload>("deep-link-message", (event) => {
    onOpen(event.payload);
  });
}

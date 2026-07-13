import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { toast } from "sonner";

import {
  inviteErrorMessage,
  isInviteExpiredError,
} from "@/shared/api/inviteHelpers";
import { claimInvite } from "@/shared/api/invites";
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
 * Payload emitted by the Rust deep-link handler for `buzz://message?…`.
 * Field names match the JSON shape produced in `desktop/src-tauri/src/lib.rs`.
 */
export type MessageDeepLinkPayload = {
  channelId: string;
  messageId: string;
  threadRootId: string | null;
};

export type NostrBindDeepLinkPayload = {
  challengeId: string;
  nonce: string;
  verificationCode: string;
  audience: "buzz:nostr-identity";
  action: "bind_nostr_identity";
  protocol: "buzz-nostr-identity";
  version: "1";
  origin: string;
  expiresAt: string;
  returnMode: "clipboard";
};

/**
 * Payload emitted by the Rust deep-link handler for `buzz://join?…` —
 * a relay invite from the web landing page (`/invite/<code>`).
 */
export type JoinDeepLinkPayload = {
  relayUrl: string;
  code: string;
};

/**
 * Register listeners for deep-link events emitted by the Rust backend.
 *
 * When a `buzz://connect?relay=<url>` link is opened, the handler
 * adds a workspace for the relay (deduplicating by URL) and switches
 * to it. Returns an unlisten function to tear down all listeners.
 *
 * When a `buzz://join?relay=<url>&code=<invite>` link is opened (relay
 * invite landing page), the handler first claims the invite against the
 * relay's HTTP API — signed by this app's identity key — and only adds and
 * switches to the workspace once the relay has admitted the key.
 *
 * `buzz://message?…` is handled separately by `listenForMessageDeepLinks`,
 * because it needs to dispatch into the router which only exists below the
 * `RouterProvider` in the component tree.
 */
export function listenForDeepLinks(deps: DeepLinkDeps): Promise<UnlistenFn> {
  const addAndSwitch = (rawRelayUrl: string) => {
    const relayUrl = normalizeRelayUrl(rawRelayUrl);
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
    return name;
  };

  const connectPromise = listen<string>("deep-link-connect", (event) => {
    const name = addAndSwitch(event.payload);
    toast.success(`Connected to ${name}`);
  });

  const joinPromise = listen<JoinDeepLinkPayload>("deep-link-join", (event) => {
    const { relayUrl, code } = event.payload;
    void claimInvite(relayUrl, code)
      .then((result) => {
        const name = addAndSwitch(relayUrl);
        toast.success(
          result.status === "already_member"
            ? `Already a member of ${name}`
            : `Joined ${name}`,
        );
      })
      .catch((error: unknown) => {
        const message = inviteErrorMessage(error);
        toast.error(
          isInviteExpiredError(error)
            ? "This invite link has expired — ask for a new one."
            : `Couldn't accept the invite: ${message}`,
        );
      });
  });

  return Promise.all([connectPromise, joinPromise]).then((unlistens) => () => {
    for (const unlisten of unlistens) unlisten();
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

export function listenForNostrBindDeepLinks(
  onOpen: (payload: NostrBindDeepLinkPayload) => void,
): Promise<UnlistenFn> {
  return listen<NostrBindDeepLinkPayload>("deep-link-nostr-bind", (event) => {
    onOpen(event.payload);
  });
}

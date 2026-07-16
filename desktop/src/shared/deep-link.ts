import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { StartCommunityOnboardingInput } from "@/features/onboarding/communityOnboarding";

export interface DeepLinkDeps {
  startCommunityOnboarding: (input: StartCommunityOnboardingInput) => boolean;
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
  returnMode: "clipboard" | "browser_fragment_v1";
  callbackUrl?: string;
};

/**
 * Payload emitted by the Rust deep-link handler for `buzz://join?…` —
 * a relay invite from the web landing page (`/invite/<code>`).
 */
export type JoinDeepLinkPayload = {
  relayUrl: string;
  code: string;
  policyReceipt: string | null;
};

type PendingCommunityDeepLink = {
  id: string;
  kind: "connect" | "join";
  relayUrl: string;
  code: string | null;
  policyReceipt: string | null;
};

function acceptPendingCommunityDeepLink(
  pending: PendingCommunityDeepLink,
  deps: DeepLinkDeps,
) {
  const accepted = deps.startCommunityOnboarding({
    source: pending.kind === "join" ? "deep-link-join" : "deep-link-connect",
    relayUrl: pending.relayUrl,
    inviteCode: pending.code ?? undefined,
    policyReceipt: pending.policyReceipt ?? undefined,
  });
  return accepted
    ? invoke<boolean>("acknowledge_pending_community_deep_link", {
        id: pending.id,
      })
    : Promise.resolve(false);
}

async function drainPendingCommunityDeepLinks(deps: DeepLinkDeps) {
  while (true) {
    const pending = await invoke<PendingCommunityDeepLink | null>(
      "take_pending_community_deep_link",
    );
    if (!pending) return;
    if (!(await acceptPendingCommunityDeepLink(pending, deps))) return;
  }
}

/**
 * Register listeners for deep-link events emitted by the Rust backend.
 *
 * When a `buzz://connect?relay=<url>` link is opened, the handler
 * adds a community for the relay (deduplicating by URL) and switches
 * to it. Returns an unlisten function to tear down all listeners.
 *
 * When a `buzz://join?relay=<url>&code=<invite>` link is opened (relay
 * invite landing page), the handler first claims the invite against the
 * relay's HTTP API — signed by this app's identity key — and only adds and
 * switches to the community once the relay has admitted the key.
 *
 * `buzz://message?…` is handled separately by `listenForMessageDeepLinks`,
 * because it needs to dispatch into the router which only exists below the
 * `RouterProvider` in the component tree.
 */
export async function listenForDeepLinks(
  deps: DeepLinkDeps,
): Promise<UnlistenFn> {
  const drain = () => {
    void drainPendingCommunityDeepLinks(deps).catch((error: unknown) => {
      console.warn("Failed to drain pending community deep links", error);
    });
  };
  const connectPromise = listen<string>("deep-link-connect", drain);
  const joinPromise = listen<JoinDeepLinkPayload>("deep-link-join", drain);
  const unlistens = await Promise.all([connectPromise, joinPromise]);
  drain();
  return () => {
    for (const unlisten of unlistens) unlisten();
  };
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

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { StartCommunityOnboardingInput } from "@/features/onboarding/communityOnboarding";

export type AddCommunityDeepLinkPayload = {
  relayUrl: string;
  name?: string;
};

export interface DeepLinkDeps {
  startCommunityOnboarding: (input: StartCommunityOnboardingInput) => boolean;
  openAddCommunity: (
    payload: AddCommunityDeepLinkPayload & { requestId: string },
  ) => boolean;
  onAddCommunityAvailable: (listener: () => void) => () => void;
}

export type ChannelDeepLinkPayload = { channelId: string };

/**
 * Payload emitted by the Rust deep-link handler for `buzz://message?…`.
 * Field names match the JSON shape produced in `desktop/src-tauri/src/lib.rs`.
 */
export type MessageDeepLinkPayload = {
  channelId: string;
  messageId: string;
  threadRootId: string | null;
};

type PendingNavigationDeepLink = {
  id: string;
  kind: "channel" | "message";
  channelId: string;
  messageId: string | null;
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
  kind: "connect" | "join" | "add-community";
  relayUrl: string;
  code: string | null;
  name: string | null;
  policyReceipt: string | null;
};

type PendingEntityDeepLink = {
  id: string;
  href: string;
};

function acceptPendingCommunityDeepLink(
  pending: PendingCommunityDeepLink,
  deps: DeepLinkDeps,
) {
  const accepted =
    pending.kind === "add-community"
      ? deps.openAddCommunity({
          requestId: pending.id,
          relayUrl: pending.relayUrl,
          name: pending.name ?? undefined,
        })
      : deps.startCommunityOnboarding({
          source:
            pending.kind === "join" ? "deep-link-join" : "deep-link-connect",
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
    if (pending.kind === "add-community") return;
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
 * `buzz://message?…` is handled separately by `listenForNavigationDeepLinks`,
 * because it needs to dispatch into the router which only exists below the
 * `RouterProvider` in the component tree.
 */
export async function listenForDeepLinks(
  deps: DeepLinkDeps,
): Promise<UnlistenFn> {
  let drainRunning = false;
  let drainRequested = false;
  const drain = () => {
    drainRequested = true;
    if (drainRunning) return;
    drainRunning = true;
    void (async () => {
      try {
        while (drainRequested) {
          drainRequested = false;
          await drainPendingCommunityDeepLinks(deps);
        }
      } catch (error: unknown) {
        console.warn("Failed to drain pending community deep links", error);
      } finally {
        drainRunning = false;
        if (drainRequested) drain();
      }
    })();
  };
  const stopAvailabilityListener = deps.onAddCommunityAvailable(drain);
  const connectPromise = listen<string>("deep-link-connect", drain);
  const joinPromise = listen<JoinDeepLinkPayload>("deep-link-join", drain);
  const addCommunityPromise = listen<AddCommunityDeepLinkPayload>(
    "deep-link-add-community",
    drain,
  );
  const unlistens = await Promise.all([
    connectPromise,
    joinPromise,
    addCommunityPromise,
  ]);
  drain();
  return () => {
    stopAvailabilityListener();
    for (const unlisten of unlistens) unlisten();
  };
}

let navigationDrainTail: Promise<void> = Promise.resolve();
let navigationDrainGeneration = 0;
let navigationDrainEnabled = true;

export async function resetNavigationDeepLinkDrain(): Promise<void> {
  const generation = ++navigationDrainGeneration;
  // Fail closed while the outgoing community's native queue is being cleared.
  // A rejected clear leaves that queue's identity unknown, so no later listener
  // may route it against a different community.
  navigationDrainEnabled = false;
  await invoke("clear_pending_navigation_deep_links");
  if (generation === navigationDrainGeneration) {
    navigationDrainEnabled = true;
  }
}

function serializeNavigationDrain(task: () => Promise<void>): Promise<void> {
  const drain = navigationDrainTail.then(task, task);
  // Keep the shared tail fulfilled so one route failure cannot poison future
  // listener mounts. The caller still receives `drain` and reports the error.
  navigationDrainTail = drain.catch(() => {});
  return drain;
}

async function drainPendingNavigationDeepLinks(
  onOpenChannel: (
    payload: ChannelDeepLinkPayload,
  ) => boolean | Promise<boolean>,
  onOpenMessage: (
    payload: MessageDeepLinkPayload,
  ) => boolean | Promise<boolean>,
) {
  const generation = navigationDrainGeneration;
  if (!navigationDrainEnabled) return;
  while (navigationDrainEnabled && generation === navigationDrainGeneration) {
    const pending = await invoke<PendingNavigationDeepLink | null>(
      "take_pending_navigation_deep_link",
    );
    if (
      !pending ||
      !navigationDrainEnabled ||
      generation !== navigationDrainGeneration
    ) {
      return;
    }
    const accepted = await (pending.kind === "channel"
      ? onOpenChannel({ channelId: pending.channelId })
      : pending.messageId
        ? onOpenMessage({
            channelId: pending.channelId,
            messageId: pending.messageId,
            threadRootId: pending.threadRootId,
          })
        : false);
    if (!accepted || generation !== navigationDrainGeneration) return;
    const acknowledged = await invoke<boolean>(
      "acknowledge_pending_navigation_deep_link",
      { id: pending.id },
    );
    if (!acknowledged) return;
  }
}

/**
 * Register listeners for queued channel/message navigation emitted by Rust.
 * A consumer must explicitly accept each item before it is acknowledged, so
 * effect teardown leaves an in-flight queue head available for the next mount.
 */
export async function listenForNavigationDeepLinks(
  onOpenChannel: (
    payload: ChannelDeepLinkPayload,
  ) => boolean | Promise<boolean>,
  onOpenMessage: (
    payload: MessageDeepLinkPayload,
  ) => boolean | Promise<boolean>,
): Promise<UnlistenFn> {
  let drainRunning = false;
  let drainRequested = false;
  const drain = () => {
    drainRequested = true;
    if (drainRunning) return;
    drainRunning = true;
    void (async () => {
      try {
        while (drainRequested) {
          drainRequested = false;
          await serializeNavigationDrain(() =>
            drainPendingNavigationDeepLinks(onOpenChannel, onOpenMessage),
          );
        }
      } catch (error: unknown) {
        console.warn("Failed to drain pending navigation deep links", error);
      } finally {
        drainRunning = false;
        if (drainRequested) drain();
      }
    })();
  };

  const unlistens = await Promise.all([
    listen<ChannelDeepLinkPayload>("deep-link-channel", drain),
    listen<MessageDeepLinkPayload>("deep-link-message", drain),
  ]);
  drain();
  return () => {
    for (const unlisten of unlistens) unlisten();
  };
}

/**
 * Register a listener for `deep-link-entity` events — the `buzz://` share
 * links for projects, repositories, issues, and pull requests. The payload is
 * the raw URL; callers parse it with `parseEntityLink` before navigating.
 */
export function listenForEntityDeepLinks(
  onOpen: (href: string) => boolean,
): Promise<UnlistenFn> {
  let drainRunning = false;
  let drainRequested = false;
  const drain = () => {
    drainRequested = true;
    if (drainRunning) return;
    drainRunning = true;
    void (async () => {
      try {
        while (drainRequested) {
          drainRequested = false;
          while (true) {
            const pending = await invoke<PendingEntityDeepLink | null>(
              "take_pending_entity_deep_link",
            );
            if (!pending) break;
            if (!onOpen(pending.href)) return;
            const acknowledged = await invoke<boolean>(
              "acknowledge_pending_entity_deep_link",
              { id: pending.id },
            );
            if (!acknowledged) break;
          }
        }
      } catch (error: unknown) {
        console.warn("Failed to drain pending entity deep links", error);
      } finally {
        drainRunning = false;
        if (drainRequested) drain();
      }
    })();
  };

  return listen<PendingEntityDeepLink | string>("deep-link-entity", (event) => {
    // String payloads are retained for older backends and E2E bridge calls.
    if (typeof event.payload === "string") {
      onOpen(event.payload);
    } else {
      drain();
    }
  }).then((unlisten) => {
    drain();
    return unlisten;
  });
}

export function listenForNostrBindDeepLinks(
  onOpen: (payload: NostrBindDeepLinkPayload) => void,
): Promise<UnlistenFn> {
  return listen<NostrBindDeepLinkPayload>("deep-link-nostr-bind", (event) => {
    onOpen(event.payload);
  });
}

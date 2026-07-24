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
 * Payload emitted by the Rust deep-link handler for `buzz://import-claim?…` —
 * the last step of a zero-touch Slack→Buzz identity migration. Either the
 * email channel (`token` + `service` present) or the OIDC channel (`via` ===
 * "oidc") is present; the Rust parser rejects a link that identifies neither.
 * Field names match the camelCase JSON produced in
 * `desktop/src-tauri/src/deep_link.rs`.
 */
export type ImportClaimDeepLinkPayload = {
  /** Native queue id used to survive cold launches without duplicate delivery. */
  requestId?: string;
  /** `<source>:<foreign id>`, e.g. `slack:U060976D0QN`. */
  subject: string;
  /** Email channel: single-use magic-link token to redeem at `service`. */
  token?: string;
  /** Email channel: base URL of the operator claim-service (http/https). */
  service?: string;
  /** OIDC channel marker (`"oidc"`); the attestation is already published. */
  via?: string;
  /** OIDC join channel: community relay that received the attestation. */
  relayUrl?: string;
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
  kind: "connect" | "join" | "add-community" | "join-slack";
  relayUrl: string;
  code: string | null;
  name: string | null;
  policyReceipt: string | null;
  /** Claim-service base URL — only set for the `join-slack` kind. */
  service: string | null;
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
      : pending.kind === "join-slack"
        ? deps.startCommunityOnboarding({
            source: "deep-link-join-slack",
            relayUrl: pending.relayUrl,
            slackService: pending.service ?? undefined,
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
 * `buzz://message?…` is handled separately by `listenForMessageDeepLinks`,
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
  const joinSlackPromise = listen("deep-link-join-slack", drain);
  const addCommunityPromise = listen<AddCommunityDeepLinkPayload>(
    "deep-link-add-community",
    drain,
  );
  const unlistens = await Promise.all([
    connectPromise,
    joinPromise,
    joinSlackPromise,
    addCommunityPromise,
  ]);
  drain();
  return () => {
    stopAvailabilityListener();
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

export function listenForImportClaimDeepLinks(
  onOpen: (payload: ImportClaimDeepLinkPayload) => void,
): Promise<UnlistenFn> {
  const delivered = new Set<string>();
  const deliver = async (payload: ImportClaimDeepLinkPayload) => {
    if (payload.requestId) {
      if (delivered.has(payload.requestId)) return;
      delivered.add(payload.requestId);
      onOpen(payload);
      await invoke<boolean>("acknowledge_pending_import_claim_deep_link", {
        requestId: payload.requestId,
      });
      return;
    }
    onOpen(payload);
  };
  const drain = async () => {
    while (true) {
      const pending = await invoke<ImportClaimDeepLinkPayload | null>(
        "take_pending_import_claim_deep_link",
      );
      if (!pending) return;
      await deliver(pending);
      if (!pending.requestId) return;
    }
  };
  return listen<ImportClaimDeepLinkPayload>(
    "deep-link-import-claim",
    (event) => {
      void deliver(event.payload).catch((error: unknown) => {
        console.warn("Failed to acknowledge import-claim deep link", error);
      });
    },
  ).then((unlisten) => {
    void drain().catch((error: unknown) => {
      console.warn("Failed to drain pending import-claim deep links", error);
    });
    return unlisten;
  });
}

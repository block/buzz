/**
 * Mounted-caller regression for PR #7191 review claim 1 (review 5156991744):
 * a third-party membership change in a BACKGROUND subscribed channel must
 * refresh the viewer's mention directory.
 *
 * The seam under test is the real app-wide background receiver:
 * `useLiveChannelUpdates`' `handleIncomingMessage` — the per-channel live
 * subscription callback every channel row (kind 40099 included) already arrives
 * through (AppShell → useUnreadChannels → useLiveChannelUpdates). The
 * mounted-channel refresh (messages/hooks.ts) only runs for the channel on
 * screen, and the global membership hook (useMembershipNotifications) only
 * sees viewer-addressed 44100/44101 rows; neither covers this row, so before
 * the fix the directory stayed stale until the 5-minute focused poll.
 *
 * Falsifiability: every assertion is driven by the ordinary relay wire shape —
 * relay-keypair-signed kind:40099 rows with `{"type":"member_joined"|
 * "member_left"|"member_removed","actor":A,"target":T}` content and an h tag
 * (crates/buzz-relay/src/handlers/side_effects.rs: emit_system_message /
 * handle_put_user / handle_remove_user). Deleting the
 * `refreshDirectoryAfterMembershipChange` call in `handleIncomingMessage`
 * fails these tests; a non-membership 40099 row must NOT refresh. The emit
 * helper refuses to deliver a row the subscription filter would not carry, so
 * the fixture cannot pass through a kind the real hook never receives.
 *
 * Harness shape: same pattern as useCommunityJoinAlerts.test.mjs — minimal
 * DOM shim → __TAURI_INTERNALS__ interception → production imports →
 * createRoot/act inside a QueryClientProvider. relayClient's subscription
 * entry points are replaced with mock.method so no socket is opened.
 */

import assert from "node:assert/strict";
import { afterEach, beforeEach, describe, it, mock } from "node:test";

// ── Minimal DOM shim ─────────────────────────────────────────────────────────

function installDOMShim() {
  class MinimalEventTarget {
    constructor() {
      this._listeners = {};
    }
    addEventListener(type, fn) {
      if (!this._listeners[type]) this._listeners[type] = [];
      this._listeners[type].push(fn);
    }
    removeEventListener(type, fn) {
      if (this._listeners[type]) {
        this._listeners[type] = this._listeners[type].filter((f) => f !== fn);
      }
    }
    dispatchEvent(e) {
      for (const fn of this._listeners[e.type] ?? []) fn(e);
      return true;
    }
  }

  class MinimalNode extends MinimalEventTarget {
    constructor(tagName) {
      super();
      this.tagName = tagName;
      this.children = [];
      this.childNodes = [];
      this.style = {};
      this.nodeType = 1;
      this.parentNode = null;
    }
    get ownerDocument() {
      return globalThis.document;
    }
    get firstChild() {
      return this.children[0] ?? null;
    }
    get lastChild() {
      return this.children[this.children.length - 1] ?? null;
    }
    get nextSibling() {
      return null;
    }
    get nodeValue() {
      return null;
    }
    appendChild(child) {
      this.children.push(child);
      this.childNodes.push(child);
      child.parentNode = this;
      return child;
    }
    removeChild(child) {
      this.children = this.children.filter((c) => c !== child);
      this.childNodes = this.childNodes.filter((c) => c !== child);
      return child;
    }
    insertBefore(newNode, refNode) {
      if (!refNode) return this.appendChild(newNode);
      const i = this.children.indexOf(refNode);
      if (i < 0) return this.appendChild(newNode);
      this.children.splice(i, 0, newNode);
      this.childNodes.splice(i, 0, newNode);
      newNode.parentNode = this;
      return newNode;
    }
    contains(node) {
      if (!node) return false;
      return this === node || this.children.some((c) => c?.contains?.(node));
    }
  }

  class MinimalDocument extends MinimalEventTarget {
    constructor() {
      super();
      this.nodeType = 9;
    }
    createElement(tagName) {
      return new MinimalNode(tagName);
    }
    createTextNode(value) {
      const n = new MinimalNode("#text");
      n.nodeValue = value;
      n.nodeType = 3;
      return n;
    }
    createComment(value) {
      const n = new MinimalNode("#comment");
      n.nodeValue = value;
      n.nodeType = 8;
      return n;
    }
    get body() {
      if (!this._body) this._body = this.createElement("body");
      return this._body;
    }
    get activeElement() {
      return null;
    }
    contains(node) {
      return node != null;
    }
  }

  globalThis.document = new MinimalDocument();
  globalThis.HTMLElement = MinimalNode;
  // react-dom's commit phase does `element instanceof window.HTMLIFrameElement`
  // (getActiveElementDeep). Leaving it undefined throws out of commitRoot.
  globalThis.HTMLIFrameElement = MinimalNode;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";

  if (typeof globalThis.window === "undefined") {
    Object.defineProperty(globalThis, "window", {
      value: globalThis,
      configurable: true,
    });
  }
  if (!Object.getOwnPropertyDescriptor(globalThis, "navigator")?.value) {
    Object.defineProperty(globalThis, "navigator", {
      value: { userAgent: "node" },
      configurable: true,
    });
  }
  globalThis.MutationObserver = class {
    observe() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  };
  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);
}

installDOMShim();

// ── localStorage shim ────────────────────────────────────────────────────────

const storage = new Map();
globalThis.localStorage = {
  get length() {
    return storage.size;
  },
  key: (index) => [...storage.keys()][index] ?? null,
  getItem: (key) => storage.get(key) ?? null,
  setItem: (key, value) => storage.set(key, value),
  removeItem: (key) => storage.delete(key),
  clear: () => storage.clear(),
};
globalThis.window.localStorage = globalThis.localStorage;

// ── Tauri IPC interceptor ────────────────────────────────────────────────────
//
// The seam under test runs entirely on the query cache, so any Tauri command
// reaching this harness means the fixture drifted off the intended path. Fail
// loudly rather than silently letting a stray IPC call pass.

globalThis.__TAURI_INTERNALS__ = {
  invoke: (cmd) =>
    Promise.reject(new Error(`unexpected Tauri command on this seam: ${cmd}`)),
  transformCallback: () => Math.random(),
};

// ── Production imports (after shims) ─────────────────────────────────────────

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";
import {
  QueryClient,
  QueryClientProvider,
  QueryObserver,
} from "@tanstack/react-query";

import { useLiveChannelUpdates } from "./useLiveChannelUpdates.ts";
import { resetMembershipDirectorySync } from "./membershipDirectorySync.ts";
import { relayClient } from "@/shared/api/relayClient.ts";
import {
  KIND_STREAM_MESSAGE,
  KIND_SYSTEM_MESSAGE,
} from "@/shared/constants/kinds";

// ── Constants ────────────────────────────────────────────────────────────────

const VIEWER = "a".repeat(64); // subscribed viewer, viewing another channel
const ACTOR = "b".repeat(64); // A: the member performing the change
const TARGET = "c".repeat(64); // T: the member being added/removed
const RELAY_KEY = "f".repeat(64); // emit_system_message signs 40099 rows
const BACKGROUND_CHANNEL = "11111111-1111-4111-8111-111111111111";
const ACTIVE_CHANNEL = "22222222-2222-4222-8222-222222222222";
const DIRECTORY_KEY = ["relay-agents"];

function channelFixture(id, name) {
  return {
    id,
    name,
    channelType: "stream",
    visibility: "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 2,
    memberPubkeys: [VIEWER, ACTOR],
    lastMessageAt: null,
    archivedAt: null,
    participants: [VIEWER, ACTOR],
    participantPubkeys: [VIEWER, ACTOR],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

/**
 * The ordinary relay wire shape for a third-party membership change.
 * `handle_put_user` (A adds T) emits member_joined; `handle_remove_user`
 * emits member_removed (third party) or member_left (self). Actor and target
 * ride in the content JSON; the row is signed by the relay keypair and
 * addressed to the channel with its h tag.
 */
function membershipEvent({ id, type, channelId = BACKGROUND_CHANNEL }) {
  return {
    id,
    kind: KIND_SYSTEM_MESSAGE,
    pubkey: RELAY_KEY,
    created_at: 2_000,
    content: JSON.stringify({ type, actor: ACTOR, target: TARGET }),
    tags: [["h", channelId]],
    sig: "s".repeat(128),
  };
}

/**
 * Replace relayClient's subscription entry points and hand the test direct
 * control of the per-channel live callbacks the hook registers.
 */
function installRelayStub() {
  /** @type {Map<string, Array<(event: unknown) => void>>} */
  const liveByChannel = new Map();
  const subscribedKindsByChannel = new Map();
  const reconnectListeners = [];

  mock.method(relayClient, "subscribeLive", async (filter, onEvent) => {
    const channelId = filter["#h"][0];
    if (!liveByChannel.has(channelId)) liveByChannel.set(channelId, []);
    liveByChannel.get(channelId).push(onEvent);
    subscribedKindsByChannel.set(channelId, new Set(filter.kinds));
    return async () => {
      const list = liveByChannel.get(channelId) ?? [];
      liveByChannel.set(
        channelId,
        list.filter((fn) => fn !== onEvent),
      );
    };
  });

  mock.method(relayClient, "subscribeToReconnects", (listener) => {
    reconnectListeners.push(listener);
    return () => {
      const i = reconnectListeners.indexOf(listener);
      if (i >= 0) reconnectListeners.splice(i, 1);
    };
  });

  // Not armed in this fixture (no onLiveMention), but mocked so an unexpected
  // arm stays hermetic instead of opening a socket.
  mock.method(
    relayClient,
    "subscribeToChannelMentionEvents",
    async () => async () => {},
  );

  return {
    /** Live callbacks registered for a channel — the subscription precondition. */
    liveSubCount: (channelId) => (liveByChannel.get(channelId) ?? []).length,
    /**
     * Deliver an event down a channel's live callbacks, refusing rows the
     * subscription filter would not carry — the fixture must be a row this
     * hook really receives.
     */
    emitLive: (channelId, event) => {
      const kinds = subscribedKindsByChannel.get(channelId);
      assert.ok(
        kinds?.has(event.kind),
        `fixture row kind ${event.kind} is not in the live subscription filter for ${channelId}`,
      );
      for (const fn of liveByChannel.get(channelId) ?? []) fn(event);
    },
  };
}

// ── Mount ─────────────────────────────────────────────────────────────────────

let relay = null;
let queryClient = null;
let disposeObserver = null;
let root = null;
let harnessTree = null;
let directoryRoster = ["existing-agent"];
let directoryFetches = 0;

beforeEach(() => {
  relay = installRelayStub();
  queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  directoryFetches = 0;

  // The real app-wide mount: viewer subscribes to both channels but is
  // viewing ACTIVE_CHANNEL, so BACKGROUND_CHANNEL is background.
  function Harness() {
    useLiveChannelUpdates(
      [
        channelFixture(BACKGROUND_CHANNEL, "agents"),
        channelFixture(ACTIVE_CHANNEL, "general"),
      ],
      ACTIVE_CHANNEL,
      { currentPubkey: VIEWER },
    );
    return null;
  }

  const container = document.createElement("div");
  root = createRoot(container);
  harnessTree = React.createElement(
    QueryClientProvider,
    { client: queryClient },
    React.createElement(Harness, null),
  );
});

afterEach(async () => {
  // Drop the helper's coalesce timers so no pending flush leaks into the next
  // test — same cleanup the helper's own suite performs.
  resetMembershipDirectorySync();
  await act(async () => {
    root.unmount();
  });
  disposeObserver?.();
  queryClient.clear();
});

async function settle(iterations = 4) {
  for (let i = 0; i < iterations; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
  }
}

/**
 * Mount the real hook with a live ["relay-agents"] directory observer whose
 * first read returns `initialRoster`. The roster variable is what "the relay
 * now lists" for each subsequent fetch — tests flip it to model the change
 * the refresh must pick up.
 */
async function mountWithDirectory(initialRoster) {
  directoryRoster = initialRoster;
  const observer = new QueryObserver(queryClient, {
    queryKey: DIRECTORY_KEY,
    queryFn: async () => {
      directoryFetches += 1;
      return directoryRoster;
    },
    staleTime: Infinity,
  });
  disposeObserver = observer.subscribe(() => {});

  await act(async () => {
    root.render(harnessTree);
  });
  await settle();
}

describe("useLiveChannelUpdates membership directory refresh", () => {
  it("background member_joined (A adds T) refreshes the mention directory", async () => {
    await mountWithDirectory(["existing-agent"]);
    assert.equal(relay.liveSubCount(BACKGROUND_CHANNEL), 1);
    assert.equal(directoryFetches, 1);
    assert.deepEqual(queryClient.getQueryData(DIRECTORY_KEY), [
      "existing-agent",
    ]);

    // Ordinary background A→T add arriving on the channel's live subscription.
    await act(async () => {
      relay.emitLive(
        BACKGROUND_CHANNEL,
        membershipEvent({ id: "join-1", type: "member_joined" }),
      );
    });

    // The directory query is marked stale synchronously by the new wiring.
    assert.equal(
      queryClient.getQueryState(DIRECTORY_KEY).isInvalidated,
      true,
      "member_joined row must invalidate the mention directory",
    );

    // The relay's directory now lists T; the coalesced refetch must pick it up
    // instead of waiting for the 5-minute focused poll.
    directoryRoster = ["existing-agent", TARGET];
    await settleAfterDirectoryCoalesce();

    assert.equal(directoryFetches, 2);
    assert.deepEqual(queryClient.getQueryData(DIRECTORY_KEY), [
      "existing-agent",
      TARGET,
    ]);
  });

  it("background member_removed (A removes T) refreshes the mention directory", async () => {
    await mountWithDirectory(["existing-agent", TARGET]);
    assert.equal(relay.liveSubCount(BACKGROUND_CHANNEL), 1);
    assert.equal(directoryFetches, 1);
    assert.deepEqual(queryClient.getQueryData(DIRECTORY_KEY), [
      "existing-agent",
      TARGET,
    ]);

    await act(async () => {
      relay.emitLive(
        BACKGROUND_CHANNEL,
        membershipEvent({ id: "remove-1", type: "member_removed" }),
      );
    });

    assert.equal(
      queryClient.getQueryState(DIRECTORY_KEY).isInvalidated,
      true,
      "member_removed row must invalidate the mention directory",
    );

    // The relay no longer lists T; the refreshed directory must drop the
    // removed member from mention candidates.
    directoryRoster = ["existing-agent"];
    await settleAfterDirectoryCoalesce();

    assert.equal(directoryFetches, 2);
    assert.deepEqual(queryClient.getQueryData(DIRECTORY_KEY), [
      "existing-agent",
    ]);
  });

  it("non-membership 40099 rows and plain messages do not refresh the directory", async () => {
    await mountWithDirectory(["existing-agent"]);
    assert.equal(relay.liveSubCount(BACKGROUND_CHANNEL), 1);
    assert.equal(directoryFetches, 1);

    await act(async () => {
      // An ordinary 40099 with a non-membership payload (topic_changed) —
      // must not trigger the membership refresh.
      relay.emitLive(BACKGROUND_CHANNEL, {
        ...membershipEvent({ id: "topic-1", type: "topic_changed" }),
        content: JSON.stringify({
          type: "topic_changed",
          actor: ACTOR,
          topic: "new topic",
        }),
      });
      // A plain stream message — not a membership change.
      relay.emitLive(BACKGROUND_CHANNEL, {
        id: "chat-1",
        kind: KIND_STREAM_MESSAGE,
        pubkey: ACTOR,
        created_at: 2_001,
        content: "hello",
        tags: [["h", BACKGROUND_CHANNEL]],
        sig: "s".repeat(128),
      });
    });

    await settleAfterDirectoryCoalesce();

    assert.equal(directoryFetches, 1);
    assert.equal(
      queryClient.getQueryState(DIRECTORY_KEY).isInvalidated,
      false,
      "only member_* payloads may refresh the directory",
    );
  });
});

/** Outlive the helper's 200 ms coalesce window, then drain effects. */
async function settleAfterDirectoryCoalesce() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 300));
  });
  await settle();
}

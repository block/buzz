/**
 * Tenant scoping for the publish-first detached agent wake.
 *
 * `useMentionSendFlow` no longer awaits `start_managed_agent`, so the call
 * outlives the send — and, because a community switch only remounts the React
 * subtree, it can outlive the community too. `start_managed_agent` resolves
 * the workspace relay and signing identity at execution time, so without a
 * captured scope a wake fired in community A can spawn/deploy the agent
 * against community B (carrying A's replay floor). These tests drive the real
 * hook against the real CommunitiesProvider and assert the invoke payload
 * carries the scope the backend fails closed on, including after a switch.
 */

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";

const SELF = "1".repeat(64);
const AGENT = "a".repeat(64);
// Mixed case on purpose: the backend's scope comparison is case-sensitive
// past the scheme, so the stored URL must reach it verbatim.
const RELAY_A = "wss://Tenant-A.example";
const RELAY_B = "wss://tenant-b.example";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** Every `start_managed_agent` payload seen, in call order. */
let startCalls = [];

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    localStorage: dom.window.localStorage,
    window: dom.window,
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (command, args) => {
      if (command === "get_identity") {
        return Promise.resolve({ pubkey: SELF, display_name: "Me" });
      }
      if (command === "start_managed_agent") {
        startCalls.push(args);
        return Promise.resolve({ pubkey: args.pubkey, status: "running" });
      }
      return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
    },
    transformCallback: () => 1,
  };
  globalThis.__TAURI_INTERNALS__ = dom.window.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

beforeEach(() => {
  startCalls = [];
  window.localStorage.clear();
  window.localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      {
        id: "community-a",
        name: "Tenant A",
        relayUrl: RELAY_A,
        pubkey: SELF,
        addedAt: "2026-01-01T00:00:00Z",
      },
      {
        id: "community-b",
        name: "Tenant B",
        relayUrl: RELAY_B,
        pubkey: SELF,
        addedAt: "2026-01-02T00:00:00Z",
      },
    ]),
  );
  window.localStorage.setItem("buzz-active-community-id", "community-a");
});

/**
 * Renders the real hook under the real communities provider, exposing the
 * detached-start callback alongside `switchCommunity` so a test can move the
 * active community out from under an already-captured callback.
 */
async function renderDetachedStart() {
  const { default: React } = await import("react");
  const { act, renderHook } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { CommunitiesProvider, useCommunities } = await import(
    "@/features/communities/useCommunities.tsx"
  );
  const { useDetachedAgentStart } = await import("./useDetachedAgentStart.ts");

  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { gcTime: 0 },
    },
  });
  const wrapper = ({ children }) =>
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(CommunitiesProvider, null, children),
    );
  const rendered = renderHook(
    () => ({
      startDetached: useDetachedAgentStart(),
      switchCommunity: useCommunities().switchCommunity,
    }),
    { wrapper },
  );
  // Let the identity query resolve so the signer scope is populated.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  return { act, rendered };
}

const AGENT_RECORD = { pubkey: AGENT, name: "fizz" };

test("a detached start carries the active community and identity as its scope", async () => {
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  assert.equal(startCalls.length, 1);
  assert.equal(
    startCalls[0].expectedRelayUrl,
    RELAY_A,
    "the stored relay URL must reach the case-sensitive backend check verbatim",
  );
  assert.equal(startCalls[0].expectedSignerPubkey, SELF);
  // The replay floor still rides along — scoping must not displace it.
  assert.ok(startCalls[0].replayFloorUnix > 0);
  rendered.unmount();
});

test("a start captured before a community switch keeps the pre-switch scope", async () => {
  const { act, rendered } = await renderDetachedStart();
  // What a send in flight holds: the callback from the render that fired it.
  const capturedStart = rendered.result.current.startDetached;

  await act(async () => {
    rendered.result.current.switchCommunity("community-b");
  });
  assert.notEqual(
    rendered.result.current.startDetached,
    capturedStart,
    "the switch must produce a new callback, so the captured one is stale",
  );

  await act(async () => {
    capturedStart(AGENT_RECORD);
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  assert.equal(startCalls.length, 1);
  assert.equal(
    startCalls[0].expectedRelayUrl,
    RELAY_A,
    "the stale wake must name the community it was fired in, not the active one",
  );
  rendered.unmount();
});

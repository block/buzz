/**
 * Tenant scoping and in-flight deduplication for the publish-first detached
 * agent wake.
 *
 * `useMentionSendFlow` no longer awaits `start_managed_agent`, so the call
 * outlives the send — and, because a community switch only remounts the React
 * subtree, it can outlive the community too. Two consequences are pinned here:
 *
 * 1. `start_managed_agent` resolves the workspace relay and signing identity at
 *    execution time, so without a captured scope a wake fired in community A
 *    can spawn/deploy the agent against community B (carrying A's replay
 *    floor).
 * 2. Nothing else dedupes concurrent wakes any more — the awaited version was
 *    covered by the composer's `isPending` gate plus the mutation's success
 *    cache write, and for the whole detached window the cache still reads
 *    `stopped`, so a second send re-fires.
 *
 * These tests drive the real hook against the real CommunitiesProvider.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";

const SELF = "1".repeat(64);
const AGENT = "a".repeat(64);
const OTHER_AGENT = "b".repeat(64);
// Mixed case on purpose: the backend's scope comparison is case-sensitive
// past the scheme, so the stored URL must reach it verbatim.
const RELAY_A = "wss://Tenant-A.example";
const RELAY_B = "wss://tenant-b.example";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** Every `start_managed_agent` payload seen, in call order. */
let startCalls = [];
/**
 * Settlers for `start_managed_agent` calls held open by `holdStarts`, in call
 * order. A held start is what the dedupe exists for: the seconds-long window
 * where a cold spawn or first deploy has not yet updated the agent record.
 */
let heldStarts = [];
let holdStarts = false;

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
        const started = { pubkey: args.pubkey, status: "running" };
        if (!holdStarts) return Promise.resolve(started);
        return new Promise((resolve, reject) => {
          heldStarts.push({
            resolve: () => resolve(started),
            reject: () => reject(new Error("spawn failed")),
          });
        });
      }
      return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
    },
    transformCallback: () => 1,
  };
  globalThis.__TAURI_INTERNALS__ = dom.window.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

afterEach(async () => {
  // Tests that pin suppression leave their wake deliberately in flight, and
  // node:test never finishes a file with a promise that will not settle.
  const outstanding = heldStarts;
  heldStarts = [];
  for (const held of outstanding) held.resolve();
  await new Promise((resolve) => setTimeout(resolve, 0));
});

beforeEach(async () => {
  startCalls = [];
  heldStarts = [];
  holdStarts = false;
  // The in-flight map is a module singleton, so a start held open by one test
  // would otherwise suppress the next test's.
  const { resetDetachedAgentStarts } = await import(
    "./useDetachedAgentStart.ts"
  );
  resetDetachedAgentStarts();
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
const OTHER_AGENT_RECORD = { pubkey: OTHER_AGENT, name: "buzz" };

/** Lets queued microtasks (the mutation, and the map's `finally`) run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

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

test("a second wake for the same agent is suppressed while the first is in flight", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();
  let first;
  let second;

  await act(async () => {
    first = rendered.result.current.startDetached(AGENT_RECORD);
    second = rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.equal(startCalls.length, 1, "one wake serves both messages");
  assert.equal(first, true);
  assert.equal(
    second,
    false,
    "the suppressed call must report that it fired nothing, so the send-perf summary does not count a wake that never happened",
  );
  rendered.unmount();
});

test("wakes for different agents in one window are not collapsed", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    rendered.result.current.startDetached(OTHER_AGENT_RECORD);
    await settle();
  });

  assert.deepEqual(
    startCalls.map((call) => call.pubkey),
    [AGENT, OTHER_AGENT],
    "the key is per agent, not a global lock on wakes",
  );
  rendered.unmount();
});

test("a wake fires again once the previous one has settled", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await act(async () => {
    heldStarts[0].resolve();
    await settle();
  });
  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.equal(startCalls.length, 2, "suppression must not outlive the start");
  rendered.unmount();
});

test("a failed wake clears the key instead of latching the agent", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await act(async () => {
    heldStarts[0].reject();
    await settle();
  });
  // The user saw "your message was sent, but the agent may not respond" and
  // retries; clearing on success only would refuse every retry for the session.
  await act(async () => {
    assert.equal(rendered.result.current.startDetached(AGENT_RECORD), true);
    await settle();
  });

  assert.equal(startCalls.length, 2);
  rendered.unmount();
});

test("a wake for the same agent in another community is not suppressed", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await act(async () => {
    rendered.result.current.switchCommunity("community-b");
  });
  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.deepEqual(
    startCalls.map((call) => call.expectedRelayUrl),
    [RELAY_A, RELAY_B],
    "the key carries the relay, so one tenant's in-flight wake never suppresses another's",
  );
  rendered.unmount();
});

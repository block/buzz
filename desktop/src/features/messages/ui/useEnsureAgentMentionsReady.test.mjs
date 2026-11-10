/**
 * The mentioned-agent readiness pass queues detached wakes instead of firing
 * them (PR #7154 review point 1). A wake fired during send preparation runs
 * before the relay has accepted the publish, so a fast start rejection could
 * toast "your message was sent" while the send outcome was unknown — and any
 * abort after the wake stranded a started harness for a message that never
 * landed. These tests pin the queue contract: nothing starts during the
 * pass, every mentioned agent that is not up lands in `agentsToWake`, and
 * each entry's replay floor is stamped at enqueue time.
 */

import assert from "node:assert/strict";
import { after, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";

const MEMBER_AGENT = "a".repeat(64);
const OTHER_AGENT = "b".repeat(64);
const CHANNEL_ID = "11111111-2222-3333-4444-555555555555";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** Every backend command reached during a test — the pass must reach none. */
let tauriInvocations = [];

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    localStorage: dom.window.localStorage,
    window: dom.window,
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (command) => {
      tauriInvocations.push(command);
      return Promise.reject(new Error(`unexpected Tauri command: ${command}`));
    },
    transformCallback: () => 1,
  };
  globalThis.__TAURI_INTERNALS__ = dom.window.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

beforeEach(() => {
  tauriInvocations = [];
});

function managedAgent(overrides = {}) {
  return {
    pubkey: MEMBER_AGENT,
    name: "fizz",
    personaId: null,
    status: "stopped",
    backend: { type: "local" },
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

/** Renders the real hook with injected seams; `overrides` vary per test. */
async function renderEnsureReady(overrides = {}) {
  const { renderHook } = await import("@testing-library/react");
  const { useEnsureAgentMentionsReady } = await import(
    "./useEnsureAgentMentionsReady.ts"
  );
  const options = {
    attachAgentToChannel: async () => {
      throw new Error("attach must not run for an existing member");
    },
    getManagedAgentsByPubkey: async () => new Map(),
    getPersonas: async () => [],
    memberPubkeys: new Set(),
    ...overrides,
  };
  return renderHook(() => useEnsureAgentMentionsReady(options));
}

test("a stopped member agent is queued for a post-publish wake, never fired", async () => {
  const agent = managedAgent();
  const rendered = await renderEnsureReady({
    getManagedAgentsByPubkey: async () => new Map([[MEMBER_AGENT, agent]]),
    memberPubkeys: new Set([MEMBER_AGENT]),
  });

  const floorLowerBound = Math.floor(Date.now() / 1000);
  const result = await rendered.result.current([MEMBER_AGENT], CHANNEL_ID);
  const floorUpperBound = Math.floor(Date.now() / 1000);

  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.pubkeys, [MEMBER_AGENT]);
  assert.equal(result.agentsToWake.length, 1);
  assert.equal(result.agentsToWake[0].agent, agent);
  // Stamped at enqueue time — before the publish — so the floor can never
  // exceed the published message's created_at, however long the flush waits.
  assert.ok(
    result.agentsToWake[0].replayFloorUnix >= floorLowerBound &&
      result.agentsToWake[0].replayFloorUnix <= floorUpperBound,
    "the replay floor must be the enqueue-time capture",
  );
  assert.deepEqual(
    tauriInvocations,
    [],
    "the readiness pass must not start the agent (or touch the backend)",
  );
  rendered.unmount();
});

test("only agents that are not up are queued", async () => {
  const runningLocal = managedAgent({ status: "running" });
  const undeployedProvider = managedAgent({
    pubkey: OTHER_AGENT,
    name: "portal",
    status: "not_deployed",
    backend: { type: "provider", id: "portal", config: {} },
  });
  const rendered = await renderEnsureReady({
    getManagedAgentsByPubkey: async () =>
      new Map([
        [MEMBER_AGENT, runningLocal],
        [OTHER_AGENT, undeployedProvider],
      ]),
    memberPubkeys: new Set([MEMBER_AGENT, OTHER_AGENT]),
  });

  const result = await rendered.result.current(
    [MEMBER_AGENT, OTHER_AGENT],
    CHANNEL_ID,
  );

  assert.deepEqual(result.errors, []);
  assert.deepEqual(
    result.agentsToWake.map((wake) => wake.agent.pubkey),
    [OTHER_AGENT],
    "a running agent needs no wake; a not-deployed provider agent does",
  );
  rendered.unmount();
});

test("a non-member agent's wake queues through the attach seam instead of firing", async () => {
  const agent = managedAgent();
  const attachCalls = [];
  const rendered = await renderEnsureReady({
    attachAgentToChannel: async (input) => {
      attachCalls.push(input);
      // The production attach invokes detachedStart for a not-up agent once
      // its membership write lands; the collector must queue, not start.
      input.detachedStart(input.agent);
      return {};
    },
    getManagedAgentsByPubkey: async () => new Map([[MEMBER_AGENT, agent]]),
  });

  const result = await rendered.result.current([MEMBER_AGENT], CHANNEL_ID);

  assert.equal(attachCalls.length, 1);
  assert.equal(attachCalls[0].channelId, CHANNEL_ID);
  assert.equal(
    result.wroteRelayState,
    true,
    "the awaited membership write still marks the publish-boundary pass",
  );
  assert.deepEqual(
    result.agentsToWake.map((wake) => wake.agent.pubkey),
    [MEMBER_AGENT],
  );
  assert.ok(result.agentsToWake[0].replayFloorUnix > 0);
  assert.deepEqual(
    tauriInvocations,
    [],
    "no start may fire while the send is still preparing",
  );
  rendered.unmount();
});

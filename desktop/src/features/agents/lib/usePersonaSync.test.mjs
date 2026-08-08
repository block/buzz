import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_DELETION,
  KIND_MANAGED_AGENT,
  KIND_PERSONA,
  KIND_PRIVATE_MANAGED_AGENT,
  KIND_TEAM,
} from "@/shared/constants/kinds";
import {
  fetchPersonaSyncBackfill,
  startPersonaSync,
} from "./usePersonaSync.ts";

const EXPECTED_KINDS = [
  KIND_PERSONA,
  KIND_TEAM,
  KIND_MANAGED_AGENT,
  KIND_PRIVATE_MANAGED_AGENT,
  KIND_DELETION,
];

function mockConnectedRelay() {
  return mock.method(relayClient, "subscribeToConnectionState", (listener) => {
    listener("connected");
    return () => {};
  });
}

test("fetchPersonaSyncBackfill drains timestamp boundaries before paging", async () => {
  const filters = [];
  const firstPage = Array.from({ length: 500 }, (_, index) => ({
    id: `head-${index}`,
    created_at: index === 499 ? 10 : 20,
  }));
  const pages = [
    firstPage,
    [
      { id: "head-499", created_at: 10 },
      { id: "boundary-peer", created_at: 10 },
    ],
    [{ id: "older", created_at: 9 }],
  ];

  const events = await fetchPersonaSyncBackfill("owner-pubkey", (filter) => {
    filters.push(filter);
    return Promise.resolve(pages.shift() ?? []);
  });

  assert.equal(events.length, 502);
  assert.deepEqual(filters[1], {
    kinds: EXPECTED_KINDS,
    authors: ["owner-pubkey"],
    limit: 500,
    since: 10,
    until: 10,
  });
  assert.equal(filters[2].until, 9);
});

test("fetchPersonaSyncBackfill rejects an unexhausted timestamp bucket", async () => {
  const fullPage = Array.from({ length: 500 }, (_, index) => ({
    id: `event-${index}`,
    created_at: 10,
  }));

  await assert.rejects(
    fetchPersonaSyncBackfill("owner-pubkey", () => Promise.resolve(fullPage)),
    /too many events share one timestamp/,
  );
});

// Regression guard for the fresh-start backfill gap (F3): a device that comes
// online AFTER another published gets zero history from a live-only `limit: 0`
// subscription, because reconnect-replay's since-cursor is undefined until the
// first live event. `startPersonaSync` MUST do a one-shot history fetch up
// front, and both the backfill and the live sub MUST carry the deletion kind
// so tombstones catch up too.
test("startPersonaSync backfills history including the deletion kind", async () => {
  const fetchCalls = [];
  const liveCalls = [];
  globalThis.window = {
    __TAURI_INTERNALS__: { invoke: () => Promise.resolve() },
  };
  mock.method(relayClient, "fetchEvents", (filter) => {
    fetchCalls.push(filter);
    return Promise.resolve([]);
  });
  mock.method(relayClient, "subscribeLive", (filter) => {
    liveCalls.push(filter);
    return Promise.resolve(() => Promise.resolve());
  });
  mockConnectedRelay();

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(fetchCalls.length, 1, "must do exactly one backfill fetch");
  assert.deepEqual(
    fetchCalls[0].kinds,
    EXPECTED_KINDS,
    "backfill must cover persona/team/agent + deletion",
  );
  assert.ok(
    fetchCalls[0].limit > 0,
    "backfill must request a positive limit — limit:0 returns no history",
  );
  assert.deepEqual(fetchCalls[0].authors, ["owner-pubkey"]);

  assert.equal(liveCalls.length, 1);
  assert.deepEqual(
    liveCalls[0].kinds,
    EXPECTED_KINDS,
    "live sub must also carry the deletion kind",
  );

  mock.reset();
  delete globalThis.window;
});

// Regression guard for the arrival-scope fix (F6): the reconcile must carry the
// relay this subscription was opened on, NOT whichever community happens to be
// active when the reconcile runs. Without the forwarded URL the backend falls
// back to the active workspace and an in-flight event lands in the wrong
// community's scoped retention store on a mid-flight switch.
test("startPersonaSync forwards its own relay as the event arrival relay", async () => {
  const invokes = [];
  // @tauri-apps/api/core reads `window.__TAURI_INTERNALS__.invoke`.
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return Promise.resolve();
      },
    },
  };

  const ownEvent = { id: "e1", pubkey: "owner-pubkey", kind: KIND_PERSONA };
  const foreignEvent = { id: "e2", pubkey: "someone-else", kind: KIND_PERSONA };

  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([ownEvent, foreignEvent]),
  );
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );
  mockConnectedRelay();

  startPersonaSync("owner-pubkey", "wss://community-a.example", () => false);
  // Let the backfill promise chain and the reconcile invoke settle.
  await new Promise((resolve) => setImmediate(resolve));

  const reconciles = invokes.filter(
    (call) => call.cmd === "reconcile_inbound_persona_event",
  );
  assert.equal(
    reconciles.length,
    1,
    "only the subscribed author's event reconciles",
  );
  assert.equal(
    reconciles[0].args.arrivalRelayUrl,
    "wss://community-a.example",
    "reconcile must carry the subscription's relay as the arrival relay",
  );
  assert.equal(JSON.parse(reconciles[0].args.eventJson).id, "e1");

  mock.reset();
  delete globalThis.window;
});

test("managed-agent restore waits for every backfill event to reconcile", async () => {
  const invokes = [];
  const pendingReconciles = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        if (cmd !== "reconcile_inbound_persona_event") {
          return Promise.resolve();
        }
        return new Promise((resolve) => pendingReconciles.push(resolve));
      },
    },
  };

  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      { id: "first", pubkey: "owner-pubkey", kind: KIND_PRIVATE_MANAGED_AGENT },
      { id: "second", pubkey: "owner-pubkey", kind: KIND_MANAGED_AGENT },
    ]),
  );
  mockConnectedRelay();

  startPersonaSync("owner-pubkey", "wss://community-a.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(pendingReconciles.length, 1, "reconciliation is serialized");
  assert.equal(
    invokes.some((call) => call.cmd === "complete_managed_agent_bootstrap"),
    false,
    "restore must not start while the first retained event is pending",
  );

  pendingReconciles.shift()();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    pendingReconciles.length,
    1,
    "second event starts after the first",
  );
  assert.equal(
    invokes.some((call) => call.cmd === "complete_managed_agent_bootstrap"),
    false,
    "restore must not start while any retained event is pending",
  );

  pendingReconciles.shift()();
  await new Promise((resolve) => setImmediate(resolve));
  const completions = invokes.filter(
    (call) => call.cmd === "complete_managed_agent_bootstrap",
  );
  assert.equal(completions.length, 1);
  assert.deepEqual(completions[0].args, {
    ownerPubkey: "owner-pubkey",
    arrivalRelayUrl: "wss://community-a.example",
  });

  mock.reset();
  delete globalThis.window;
});

test("managed-agent restore drains a live event received during backfill", async () => {
  const invokes = [];
  const pendingReconciles = [];
  let liveListener = () => {};
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        if (cmd !== "reconcile_inbound_persona_event") {
          return Promise.resolve();
        }
        return new Promise((resolve) => pendingReconciles.push(resolve));
      },
    },
  };
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    liveListener = listener;
    return Promise.resolve(() => Promise.resolve());
  });
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        id: "backfill",
        pubkey: "owner-pubkey",
        kind: KIND_PRIVATE_MANAGED_AGENT,
      },
    ]),
  );
  mockConnectedRelay();

  startPersonaSync("owner-pubkey", "wss://community-a.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(pendingReconciles.length, 1);

  liveListener({
    id: "live-during-backfill",
    pubkey: "owner-pubkey",
    kind: KIND_PRIVATE_MANAGED_AGENT,
  });
  pendingReconciles.shift()();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    pendingReconciles.length,
    1,
    "the buffered live event must reconcile before restore",
  );
  assert.equal(
    invokes.some((call) => call.cmd === "complete_managed_agent_bootstrap"),
    false,
  );

  pendingReconciles.shift()();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    invokes.filter((call) => call.cmd === "complete_managed_agent_bootstrap")
      .length,
    1,
  );

  mock.reset();
  delete globalThis.window;
});

test("managed-agent restore fails closed when the live bootstrap buffer overflows", async () => {
  const invokes = [];
  let liveListener = () => {};
  let finishBackfill = () => {};
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return Promise.resolve();
      },
    },
  };
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    liveListener = listener;
    return Promise.resolve(() => Promise.resolve());
  });
  mock.method(
    relayClient,
    "fetchEvents",
    () =>
      new Promise((resolve) => {
        finishBackfill = () => resolve([]);
      }),
  );
  mockConnectedRelay();

  startPersonaSync("owner-pubkey", "wss://community-a.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));
  for (let index = 0; index <= 20_000; index += 1) {
    liveListener({
      id: `live-${index}`,
      pubkey: "owner-pubkey",
      kind: KIND_PRIVATE_MANAGED_AGENT,
    });
  }
  finishBackfill();
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(
    invokes.some((call) => call.cmd === "complete_managed_agent_bootstrap"),
    false,
  );

  mock.reset();
  delete globalThis.window;
});

test("failed backfill reconciliation keeps managed-agent restore paused", async () => {
  const invokes = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return cmd === "reconcile_inbound_persona_event"
          ? Promise.reject(new Error("retention unavailable"))
          : Promise.resolve();
      },
    },
  };
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );
  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([
      {
        id: "broken",
        pubkey: "owner-pubkey",
        kind: KIND_PRIVATE_MANAGED_AGENT,
      },
    ]),
  );
  mockConnectedRelay();

  startPersonaSync("owner-pubkey", "wss://community-a.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    invokes.some((call) => call.cmd === "complete_managed_agent_bootstrap"),
    false,
  );

  mock.reset();
  delete globalThis.window;
});

test("managed-agent bootstrap retries after the relay reconnects", async () => {
  const invokes = [];
  let connectionListener = () => {};
  let fetchAttempt = 0;
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return Promise.resolve();
      },
    },
  };
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );
  mock.method(relayClient, "subscribeToConnectionState", (listener) => {
    connectionListener = listener;
    listener("connected");
    return () => {};
  });
  mock.method(relayClient, "fetchEvents", () => {
    fetchAttempt += 1;
    return fetchAttempt === 1
      ? Promise.reject(new Error("relay unavailable"))
      : Promise.resolve([]);
  });

  startPersonaSync("owner-pubkey", "wss://community-a.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    invokes.some((call) => call.cmd === "complete_managed_agent_bootstrap"),
    false,
  );

  connectionListener("reconnecting");
  connectionListener("connected");
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    invokes.filter((call) => call.cmd === "complete_managed_agent_bootstrap")
      .length,
    1,
  );

  mock.reset();
  delete globalThis.window;
});

test("live agent-settings sync retries after the relay reconnects", async () => {
  const invokes = [];
  let connectionListener = () => {};
  let subscribeAttempt = 0;
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return Promise.resolve();
      },
    },
  };
  mock.method(relayClient, "subscribeToConnectionState", (listener) => {
    connectionListener = listener;
    listener("connected");
    return () => {};
  });
  mock.method(relayClient, "subscribeLive", () => {
    subscribeAttempt += 1;
    return subscribeAttempt === 1
      ? Promise.reject(new Error("subscription unavailable"))
      : Promise.resolve(() => Promise.resolve());
  });
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));

  startPersonaSync("owner-pubkey", "wss://community-a.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(subscribeAttempt, 1);

  connectionListener("reconnecting");
  connectionListener("connected");
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(subscribeAttempt, 2);
  assert.equal(
    invokes.filter((call) => call.cmd === "complete_managed_agent_bootstrap")
      .length,
    1,
  );

  mock.reset();
  delete globalThis.window;
});

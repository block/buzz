import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_DELETION,
  KIND_MANAGED_AGENT,
  KIND_PERSONA,
  KIND_TEAM,
} from "@/shared/constants/kinds";
import { startPersonaSync } from "./usePersonaSync.ts";

const EXPECTED_KINDS = [
  KIND_PERSONA,
  KIND_TEAM,
  KIND_MANAGED_AGENT,
  KIND_DELETION,
];

// Regression guard for the fresh-start backfill gap (F3): a device that comes
// online AFTER another published gets zero history from a live-only `limit: 0`
// subscription, because reconnect-replay's since-cursor is undefined until the
// first live event. `startPersonaSync` MUST do an initial history fetch, and
// both the backfill and the live sub MUST carry the deletion kind so tombstones
// catch up too.
test("startPersonaSync backfills history including the deletion kind", async () => {
  const fetchCalls = [];
  const liveCalls = [];
  mock.method(relayClient, "fetchEvents", (filter) => {
    fetchCalls.push(filter);
    return Promise.resolve([]);
  });
  mock.method(relayClient, "subscribeLive", (filter) => {
    liveCalls.push(filter);
    return Promise.resolve(() => Promise.resolve());
  });

  const dispose = startPersonaSync(
    "owner-pubkey",
    "wss://relay.example",
    () => false,
  );

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

  await dispose();
  mock.reset();
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

  const dispose = startPersonaSync(
    "owner-pubkey",
    "wss://community-a.example",
    () => false,
  );
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

  await dispose();
  mock.reset();
  delete globalThis.window;
});

// Regression guard for a startup fetch that fails once while the live
// subscription remains connected. Without an independent retry, an older
// definition never arrives because the live subscription has no history
// cursor yet.
test("startPersonaSync retries a failed historical backfill", async () => {
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: () => Promise.resolve(),
    },
  };
  mock.timers.enable({ apis: ["setTimeout"] });

  let fetchCount = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetchCount += 1;
    return fetchCount === 1
      ? Promise.reject(new Error("history timed out"))
      : Promise.resolve([]);
  });
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );
  mock.method(console, "warn", () => {});

  const dispose = startPersonaSync(
    "owner-pubkey",
    "wss://relay.example",
    () => false,
  );
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(fetchCount, 1);

  mock.timers.tick(1_000);
  await Promise.resolve();
  assert.equal(fetchCount, 2, "the failed backfill must retry");

  await dispose();
  mock.timers.reset();
  mock.reset();
  delete globalThis.window;
});

test("startPersonaSync retries failed live-subscription setup", async () => {
  mock.timers.enable({ apis: ["setTimeout"] });

  let liveSubscribeCount = 0;
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "subscribeLive", () => {
    liveSubscribeCount += 1;
    return liveSubscribeCount === 1
      ? Promise.reject(new Error("subscription setup failed"))
      : Promise.resolve(() => Promise.resolve());
  });
  mock.method(console, "warn", () => {});

  const dispose = startPersonaSync(
    "owner-pubkey",
    "wss://relay.example",
    () => false,
  );
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(liveSubscribeCount, 1);

  mock.timers.tick(1_000);
  await Promise.resolve();
  assert.equal(
    liveSubscribeCount,
    2,
    "failed live-subscription setup must retry",
  );

  await dispose();
  mock.timers.reset();
  mock.reset();
});

// A relay can answer the initial history request with an incomplete view and
// recover later. Re-fetching after reconnect repairs the local definition
// store even though the missing event is older than the live subscription.
test("startPersonaSync backfills again after reconnect", async () => {
  const invokes = [];
  let reconnect;
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return Promise.resolve();
      },
    },
  };

  const recoveredEvent = {
    id: "recovered-definition",
    pubkey: "owner-pubkey",
    kind: KIND_PERSONA,
  };
  let fetchCount = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetchCount += 1;
    return Promise.resolve(fetchCount === 1 ? [] : [recoveredEvent]);
  });
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );
  mock.method(relayClient, "subscribeToReconnects", (listener) => {
    reconnect = listener;
    return () => {};
  });

  const dispose = startPersonaSync(
    "owner-pubkey",
    "wss://relay.example",
    () => false,
  );
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(fetchCount, 1);

  reconnect();
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(fetchCount, 2);
  assert.equal(
    invokes.filter((call) => call.cmd === "reconcile_inbound_persona_event")
      .length,
    1,
    "the reconnect catch-up must reconcile the previously missing definition",
  );

  await dispose();
  mock.reset();
  delete globalThis.window;
});

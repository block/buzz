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
// first live event. `startPersonaSync` MUST do a one-shot history fetch up
// front, and both the backfill and the live sub MUST carry the deletion kind
// so tombstones catch up too.
test("startPersonaSync backfills history including the deletion kind", () => {
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

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);

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

// Waits until `cond` holds, polling on a short timer. The retry loop under
// test uses injected 1ms delays, so real-time polling keeps the tests honest
// without fake timers.
async function waitFor(cond, tries = 200) {
  for (let i = 0; i < tries; i += 1) {
    if (cond()) return true;
    await new Promise((resolve) => setTimeout(resolve, 1));
  }
  return false;
}

function stubInvoke(behavior) {
  const invokes = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return behavior(invokes.length);
      },
    },
  };
  return invokes;
}

// Regression guard for the silent-forever-empty gap: a failed backfill must be
// retried, because the live subscription only covers NEW events and can never
// recover already-published persona/team/agent heads.
test("startPersonaSync retries the backfill after a fetch failure", async () => {
  const invokes = stubInvoke(() => Promise.resolve());
  const ownEvent = { id: "e1", pubkey: "owner-pubkey", kind: KIND_PERSONA };

  let fetches = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetches += 1;
    return fetches === 1
      ? Promise.reject(new Error("relay not ready"))
      : Promise.resolve([ownEvent]);
  });
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false, {
    backfillRetryDelaysMs: [1],
  });

  assert.ok(await waitFor(() => fetches === 2), "backfill must retry once");
  assert.ok(
    await waitFor(
      () =>
        invokes.filter((call) => call.cmd === "reconcile_inbound_persona_event")
          .length === 1,
    ),
    "the retried backfill must reconcile its events",
  );

  mock.reset();
  delete globalThis.window;
});

// A backfill whose fetch succeeds but whose reconciles all fail (e.g. identity
// keys not loaded yet in the backend) must also be retried — reconcile is
// idempotent, so re-applying already-landed events is a no-op.
test("startPersonaSync retries the backfill when reconciles fail", async () => {
  let invokeCalls = 0;
  const invokes = stubInvoke(() => {
    invokeCalls += 1;
    return invokeCalls === 1
      ? Promise.reject(new Error("signing keys not ready"))
      : Promise.resolve();
  });
  const ownEvent = { id: "e1", pubkey: "owner-pubkey", kind: KIND_PERSONA };

  let fetches = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetches += 1;
    return Promise.resolve([ownEvent]);
  });
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false, {
    backfillRetryDelaysMs: [1],
  });

  assert.ok(
    await waitFor(() => fetches === 2),
    "failed reconciles must trigger a backfill retry",
  );
  assert.ok(
    await waitFor(() => invokes.length === 2),
    "the retry must reconcile the same event again",
  );

  mock.reset();
  delete globalThis.window;
});

// After the retry schedule is exhausted the backfill must stop — the live
// subscription still covers new events, and an unreachable relay must not be
// polled forever.
test("startPersonaSync gives up after exhausting the retry delays", async () => {
  let fetches = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetches += 1;
    return Promise.reject(new Error("relay unreachable"));
  });
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false, {
    backfillRetryDelaysMs: [1, 1],
  });

  // initial attempt + 2 retries, then stop.
  assert.ok(await waitFor(() => fetches === 3), "must run all retries");
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(fetches, 3, "must stop after the retry schedule is exhausted");

  mock.reset();
});

// A cancelled sync (identity/community switch torn the effect down) must not
// keep retrying in the background.
test("startPersonaSync stops retrying once cancelled", async () => {
  let fetches = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetches += 1;
    return Promise.reject(new Error("relay unreachable"));
  });
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );

  startPersonaSync("owner-pubkey", "wss://relay.example", () => true, {
    backfillRetryDelaysMs: [1, 1],
  });

  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(fetches, 1, "a cancelled sync must not retry the backfill");

  mock.reset();
});

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
test("startPersonaSync backfills history including the deletion kind", async () => {
  const fetchCalls = [];
  const liveCalls = [];
  const invokes = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return Promise.resolve();
      },
    },
  };
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
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(
    invokes.filter(
      (call) => call.cmd === "mark_managed_agent_reference_sync_ready",
    ).length,
    1,
    "a completed empty backfill is explicitly marked ready",
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
  let finishReconcile;
  // @tauri-apps/api/core reads `window.__TAURI_INTERNALS__.invoke`.
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        if (cmd === "reconcile_inbound_persona_event") {
          return new Promise((resolve) => {
            finishReconcile = resolve;
          });
        }
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
  // Let the backfill reach the durable reconcile, but do not complete it yet.
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
  assert.equal(
    invokes.some(
      (call) => call.cmd === "mark_managed_agent_reference_sync_ready",
    ),
    false,
    "the identity index must remain unknown while persistence is in flight",
  );

  finishReconcile();
  await new Promise((resolve) => setImmediate(resolve));
  const ready = invokes.filter(
    (call) => call.cmd === "mark_managed_agent_reference_sync_ready",
  );
  assert.equal(ready.length, 1);
  assert.deepEqual(ready[0].args, {
    ownerPubkey: "owner-pubkey",
    arrivalRelayUrl: "wss://community-a.example",
  });

  mock.reset();
  delete globalThis.window;
});

test("startPersonaSync never marks a failed backfill ready", async () => {
  const invokes = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return Promise.resolve();
      },
    },
  };
  mock.method(relayClient, "fetchEvents", () =>
    Promise.reject(new Error("relay unavailable")),
  );
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );
  mock.method(console, "warn", () => {});

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(
    invokes.some(
      (call) => call.cmd === "mark_managed_agent_reference_sync_ready",
    ),
    false,
  );

  mock.reset();
  delete globalThis.window;
});

test("startPersonaSync treats a capped, unpageable history as unknown", async () => {
  const invokes = [];
  let fetchCount = 0;
  const tiedPage = Array.from({ length: 500 }, (_, index) => ({
    id: `agent-${index}`,
    pubkey: "owner-pubkey",
    kind: KIND_MANAGED_AGENT,
    created_at: 100,
  }));
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (cmd, args) => {
        invokes.push({ cmd, args });
        return Promise.resolve();
      },
    },
  };
  mock.method(relayClient, "fetchEvents", () => {
    fetchCount += 1;
    return Promise.resolve(tiedPage);
  });
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );
  mock.method(console, "warn", () => {});

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(fetchCount, 2, "the inclusive boundary is retried once");
  assert.equal(
    invokes.some(
      (call) => call.cmd === "mark_managed_agent_reference_sync_ready",
    ),
    false,
    "a capped history must not be treated as a complete identity index",
  );

  mock.reset();
  delete globalThis.window;
});

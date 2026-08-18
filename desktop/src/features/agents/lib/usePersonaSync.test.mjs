import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_DELETION,
  KIND_MANAGED_AGENT,
  KIND_PERSONA,
  KIND_TEAM,
} from "@/shared/constants/kinds";
import {
  coalesceManagedAgentBackfill,
  startPersonaSync,
} from "./usePersonaSync.ts";

const EXPECTED_KINDS = [
  KIND_PERSONA,
  KIND_TEAM,
  KIND_MANAGED_AGENT,
  KIND_DELETION,
];

function event({
  id,
  kind = KIND_MANAGED_AGENT,
  createdAt,
  pubkey = "owner-pubkey",
  dTag = "agent-pubkey",
}) {
  return {
    id,
    pubkey,
    created_at: createdAt,
    kind,
    tags: dTag ? [["d", dTag]] : [],
    content: "{}",
    sig: "sig",
  };
}

test("startup backfill keeps only the newest managed-agent head per coordinate", () => {
  const persona = event({
    id: "persona",
    kind: KIND_PERSONA,
    createdAt: 1,
    dTag: "persona-id",
  });
  const otherAgent = event({
    id: "other-agent",
    createdAt: 2,
    dTag: "other-agent",
  });
  const oldest = event({ id: "oldest", createdAt: 1 });
  const sameSecondLoser = event({ id: "f", createdAt: 3 });
  const newest = event({ id: "a", createdAt: 3 });

  assert.deepEqual(
    coalesceManagedAgentBackfill([
      oldest,
      persona,
      newest,
      otherAgent,
      sameSecondLoser,
    ]).map(({ id }) => id),
    ["persona", "a", "other-agent"],
    "NIP-33 uses newest created_at and lowest id on a tie",
  );
});

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

test("startPersonaSync serializes inbound reconciliation in relay order", async () => {
  const resolvers = [];
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return new Promise((resolve) => resolvers.push(resolve));
      },
    },
  };

  let onEvent;
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    onEvent = listener;
    return Promise.resolve(() => Promise.resolve());
  });

  startPersonaSync("owner-pubkey", "wss://community.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));
  onEvent({ id: "broad", pubkey: "owner-pubkey", kind: KIND_MANAGED_AGENT });
  onEvent({
    id: "restricted",
    pubkey: "owner-pubkey",
    kind: KIND_MANAGED_AGENT,
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    invokedIds,
    ["broad"],
    "newer event waits for prior deployment",
  );
  resolvers.shift()();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(invokedIds, ["broad", "restricted"]);
  resolvers.shift()();

  mock.reset();
  delete globalThis.window;
});

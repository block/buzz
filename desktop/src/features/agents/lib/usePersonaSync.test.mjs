import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_DELETION,
  KIND_MANAGED_AGENT,
  KIND_PERSONA,
  KIND_TEAM,
  KIND_TEAM_CATALOG,
} from "@/shared/constants/kinds";
import {
  coalesceManagedAgentBackfill,
  orderCatalogHeadsLast,
  PersonaHistoryDenseBoundaryError,
  startPersonaSync,
} from "./usePersonaSync.ts";

const EXPECTED_KINDS = [
  KIND_PERSONA,
  KIND_TEAM,
  KIND_MANAGED_AGENT,
  KIND_TEAM_CATALOG,
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

// Regression guard for the fresh-device backfill ordering defect (Carl r10 P1,
// finding 2): the relay serves history newest-first, so a freshly shared 30178
// head arrives before the 30176 team and 30175 personas it projects. Dispatched
// in that order, the inbound team refresh runs before device B's personas
// hydrate — member resolution fails and the owner's valid shared head is purged
// plus falsely tombstoned. `orderCatalogHeadsLast` MUST defer every catalog head
// past its constituents while preserving relay order within each group.
test("orderCatalogHeadsLast defers catalog heads past all constituents", () => {
  const catalog = event({ id: "cat", kind: KIND_TEAM_CATALOG, createdAt: 30 });
  const team = event({ id: "team", kind: KIND_TEAM, createdAt: 20 });
  const persona = event({ id: "p1", kind: KIND_PERSONA, createdAt: 10 });
  const deletion = event({ id: "del", kind: KIND_DELETION, createdAt: 5 });

  assert.deepEqual(
    // Relay newest-first order: catalog head lands before its constituents.
    orderCatalogHeadsLast([catalog, team, persona, deletion]).map(
      ({ id }) => id,
    ),
    ["team", "p1", "del", "cat"],
    "constituents dispatch first; the catalog head is deferred to the end",
  );
});

test("orderCatalogHeadsLast preserves relay order within each group", () => {
  const catA = event({ id: "cat-a", kind: KIND_TEAM_CATALOG, createdAt: 40 });
  const catB = event({ id: "cat-b", kind: KIND_TEAM_CATALOG, createdAt: 30 });
  const teamA = event({ id: "team-a", kind: KIND_TEAM, createdAt: 20 });
  const teamB = event({ id: "team-b", kind: KIND_TEAM, createdAt: 10 });

  assert.deepEqual(
    orderCatalogHeadsLast([catA, teamA, catB, teamB]).map(({ id }) => id),
    ["team-a", "team-b", "cat-a", "cat-b"],
    "a stable partition keeps newest-wins order inside constituents and heads",
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
  mock.method(relayClient, "fetchEvents", (filter) => {
    fetchCalls.push(filter);
    return Promise.resolve([]);
  });
  mock.method(relayClient, "subscribeLive", (filter) => {
    liveCalls.push(filter);
    return Promise.resolve(() => Promise.resolve());
  });

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  // Backfill runs only after the live subscription is established, so let the
  // subscribe promise resolve before asserting the fetch fired.
  for (let i = 0; i < 3; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  assert.equal(fetchCalls.length, 1, "empty first page exhausts in one fetch");
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
  assert.equal(fetchCalls[0].until, undefined, "first page carries no cursor");

  assert.equal(liveCalls.length, 1);
  assert.deepEqual(
    liveCalls[0].kinds,
    EXPECTED_KINDS,
    "live sub must also carry the deletion kind",
  );

  mock.reset();
});

// Regression guard for Thufir r10 P2 finding 2 (hydration boundary). The history
// fetch and the live subscription start concurrently into ONE reconcile chain. A
// live/replayed 30178 catalog head that arrives BEFORE the backfill has
// reconciled its 30175/30176 constituents drives the inbound team refresh against
// an unhydrated persona store — member resolution fails and the owner's valid
// witness is purged plus falsely tombstoned. `startPersonaSync` MUST buffer live
// events until the ordered backfill is dispatched, then drain them. Removing the
// buffer dispatches the live head first and turns this RED.
test("startPersonaSync buffers live catalog heads until the backfill hydrates constituents", async () => {
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return Promise.resolve();
      },
    },
  };

  let resolveBackfill;
  const backfill = new Promise((resolve) => {
    resolveBackfill = resolve;
  });
  mock.method(relayClient, "fetchEvents", () => backfill);
  let onEvent;
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    onEvent = listener;
    return Promise.resolve(() => Promise.resolve());
  });

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));

  // A freshly shared catalog head arrives live before the history resolves.
  onEvent(event({ id: "cat-live", kind: KIND_TEAM_CATALOG, createdAt: 100 }));
  // The delayed backfill returns the constituents (relay newest-first).
  resolveBackfill([
    event({ id: "team", kind: KIND_TEAM, createdAt: 90 }),
    event({ id: "persona", kind: KIND_PERSONA, createdAt: 80 }),
  ]);
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    invokedIds,
    ["team", "persona", "cat-live"],
    "constituents hydrate first; the buffered live catalog head drains last",
  );

  mock.reset();
  delete globalThis.window;
});

// Regression guard for Thufir r10 P2 finding 3 (capped page). The relay clamps a
// REQ to `max_limit` and serves newest-first, so a large owner's full history
// exceeds one 500-event page: a newer 30178/30176 can land in-page while an older
// required 30175 constituent falls beyond it. `startPersonaSync` MUST page to
// exhaustion via the `until` cursor so every constituent hydrates before the
// catalog head. Removing pagination leaves the required persona unfetched.
test("startPersonaSync pages history to exhaustion so an out-of-page constituent hydrates before the catalog head", async () => {
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return Promise.resolve();
      },
    },
  };

  // Page 1 (newest-first): the catalog head, the team, and 498 filler personas —
  // a full page whose oldest event is created_at 501.
  const page1 = [
    event({ id: "cat", kind: KIND_TEAM_CATALOG, createdAt: 1000 }),
    event({ id: "team", kind: KIND_TEAM, createdAt: 999 }),
  ];
  for (let i = 0; i < 498; i += 1) {
    page1.push(
      event({
        id: `filler-${i}`,
        kind: KIND_PERSONA,
        createdAt: 998 - i,
        dTag: `filler-${i}`,
      }),
    );
  }
  // Page 2: the boundary event re-returned by the inclusive `until`, plus the
  // required older persona that fell outside page 1.
  const boundary = page1[page1.length - 1];
  const page2 = [
    boundary,
    event({
      id: "req-persona",
      kind: KIND_PERSONA,
      createdAt: 100,
      dTag: "req-persona",
    }),
  ];

  const fetchCalls = [];
  mock.method(relayClient, "fetchEvents", (filter) => {
    fetchCalls.push(filter);
    return Promise.resolve(filter.until === undefined ? page1 : page2);
  });
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  for (let i = 0; i < 5; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  assert.equal(
    fetchCalls.length,
    2,
    "a full first page triggers a second page",
  );
  assert.equal(
    fetchCalls[1].until,
    501,
    "the second page is cursored on the oldest event",
  );
  assert.ok(
    invokedIds.includes("req-persona"),
    "the out-of-page constituent must be fetched and reconciled",
  );
  assert.ok(
    invokedIds.indexOf("req-persona") < invokedIds.indexOf("cat"),
    "the required persona hydrates before the catalog head",
  );
  assert.equal(
    invokedIds[invokedIds.length - 1],
    "cat",
    "the catalog head reconciles last, after every constituent",
  );
  assert.equal(
    invokedIds.filter((id) => id === boundary.id).length,
    1,
    "the inclusive-cursor boundary event is deduped, not reconciled twice",
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

// Regression guard for Carl r12 P1 finding 1 (startup gap between backfill and
// live registration). The live REQ only registers at the relay after
// `subscribe()`'s send completes, so if the backfill runs FIRST an owner event
// published after the history EOSE but before live registration is returned by
// neither path and is lost until remount. `startPersonaSync` MUST establish the
// live subscription first, then run the backfill — so an event arriving in that
// window is delivered live (buffered) and still reconciles. Restoring
// backfill-before-subscribe order fires `fetchEvents` before the listener
// exists, so the gap event is never delivered: reconciled zero times, RED.
test("startPersonaSync subscribes live before backfilling so a gap event still reconciles once", async () => {
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return Promise.resolve();
      },
    },
  };

  const callOrder = [];
  let onEvent;
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    callOrder.push("subscribe");
    onEvent = listener;
    return Promise.resolve(() => Promise.resolve());
  });
  // The backfill returns empty history (the gap event was published after its
  // EOSE). When the query runs, the event arrives live — deliverable ONLY
  // because the subscription already registered.
  mock.method(relayClient, "fetchEvents", () => {
    callOrder.push("fetch");
    onEvent?.(
      event({ id: "gap-event", kind: KIND_PERSONA, createdAt: 500, dTag: "g" }),
    );
    return Promise.resolve([]);
  });

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  for (let i = 0; i < 5; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    callOrder,
    ["subscribe", "fetch"],
    "the live subscription registers before the backfill queries history",
  );
  assert.deepEqual(
    invokedIds.filter((id) => id === "gap-event"),
    ["gap-event"],
    "the gap event is delivered live, buffered, and reconciled exactly once",
  );

  mock.reset();
  delete globalThis.window;
});

// Regression guard for Carl r12 P1 finding 1 (dedupe). Because the live sub now
// registers before the backfill queries history, an event published in that
// window is delivered live (buffered) AND returned by the backfill.
// `reconcileInboundPersonaEvent` is not idempotent, so the drain MUST skip any
// buffered event the backfill already reconciled. Removing the dedupe skip
// dispatches the buffered duplicate too, reconciling it twice — RED.
test("startPersonaSync reconciles an event only once when it appears both live-buffered and in the backfill", async () => {
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return Promise.resolve();
      },
    },
  };

  const overlap = event({
    id: "overlap",
    kind: KIND_PERSONA,
    createdAt: 400,
    dTag: "o",
  });
  let onEvent;
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    onEvent = listener;
    return Promise.resolve(() => Promise.resolve());
  });
  // The overlap event arrives live during the query (buffered) and is also
  // returned by the backfill history — the double-delivery window.
  mock.method(relayClient, "fetchEvents", () => {
    onEvent?.(overlap);
    return Promise.resolve([overlap]);
  });

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  for (let i = 0; i < 5; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    invokedIds.filter((id) => id === "overlap"),
    ["overlap"],
    "the backfill reconciles it once; the buffered duplicate is deduped on drain",
  );

  mock.reset();
  delete globalThis.window;
});

// Regression guard for Carl r12 P2 finding 1 (initial subscription rejection
// must not leave both paths inert). `startPersonaSync` now runs `runBackfill()`
// inside `subscribeLive(...).then(...)`. `relayClientSession.subscribe()` really
// rejects (relay unreachable, or a terminal auth state) — deleting the sub and
// rethrowing. Since the mount restarts only on `[pubkey, relayUrl]` change and
// nothing else remounts this effect, a bare `.then()` chain would leave ALL
// history unhydrated for the mount lifetime plus emit an unhandled rejection.
// The `.catch()` MUST consume the rejection and still backfill. Restoring the
// bare `.then()` (dropping the catch) fires no `fetchEvents` and leaks an
// unhandled rejection — RED.
test("startPersonaSync still backfills history when the live subscription rejects", async () => {
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return Promise.resolve();
      },
    },
  };

  const unhandled = [];
  const onUnhandled = (reason) => unhandled.push(reason);
  process.on("unhandledRejection", onUnhandled);

  mock.method(relayClient, "subscribeLive", () =>
    Promise.reject(new Error("initial subscription failed")),
  );
  // Backfill history returns one persona head; it must still reconcile even
  // though the live subscription never registered.
  const head = event({
    id: "history-head",
    kind: KIND_PERSONA,
    createdAt: 300,
    dTag: "h",
  });
  let fetchCalls = 0;
  mock.method(relayClient, "fetchEvents", () => {
    fetchCalls += 1;
    return Promise.resolve([head]);
  });

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  for (let i = 0; i < 6; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  process.removeListener("unhandledRejection", onUnhandled);

  assert.deepEqual(
    unhandled,
    [],
    "the subscription rejection is consumed, not leaked as unhandled",
  );
  assert.equal(
    fetchCalls,
    1,
    "backfill still queries history after live fails",
  );
  assert.deepEqual(
    invokedIds,
    ["history-head"],
    "the backfilled head is reconciled despite the failed live subscription",
  );

  mock.reset();
  delete globalThis.window;
});

// Regression guard for Will r10 P3 finding 1 (dense-boundary pagination). The WS
// filter exposes only a time-only `until` cursor, so when MORE than one page of
// events shares the oldest boundary second the cursor cannot advance: page 2
// returns the same slice, and older events (a required 30175) are unreachable.
// `fetchOwnerHistoryToExhaustion` MUST throw `PersonaHistoryDenseBoundaryError`
// rather than treat the unadvanceable page as exhaustion. The pipeline then
// enters degraded-live and DROPS catalog heads (their constituents never
// hydrated). Reverting to time-only `added === 0` termination silently completes
// backfill as if exhaustive: the live catalog head is reconciled (the purge
// path) instead of dropped, turning this RED.
test("startPersonaSync fails loudly on a dense boundary and degrades to catalog-dropping live sync", async () => {
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return Promise.resolve();
      },
    },
  };

  // 500 events all sharing created_at 100 — a full page whose oldest cannot
  // advance the cursor. The required older persona at 50 is unreachable behind
  // the dense second. The relay re-returns the same slice for `until: 100`.
  const densePage = [];
  for (let i = 0; i < 500; i += 1)
    densePage.push(
      event({
        id: `dense-${i}`,
        kind: KIND_PERSONA,
        createdAt: 100,
        dTag: `d-${i}`,
      }),
    );

  const fetchCalls = [];
  mock.method(relayClient, "fetchEvents", (filter) => {
    fetchCalls.push(filter);
    return Promise.resolve(densePage);
  });
  let onEvent;
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    onEvent = listener;
    return Promise.resolve(() => Promise.resolve());
  });

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  for (let i = 0; i < 5; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  assert.equal(
    fetchCalls.length,
    2,
    "a full first page pages once more, then the dense second aborts fetching",
  );
  assert.equal(
    fetchCalls[1].until,
    100,
    "the second page is cursored on the dense second",
  );

  // Degraded-live: the whole catalog dependency set is dropped (30178 head and
  // its 30175/30176 constituents), but a 30177 runtime-policy event still
  // reconciles — the subscription is not inert.
  onEvent(event({ id: "live-cat", kind: KIND_TEAM_CATALOG, createdAt: 200 }));
  onEvent(
    event({ id: "live-team", kind: KIND_TEAM, createdAt: 201, dTag: "t1" }),
  );
  onEvent(
    event({
      id: "live-agent",
      kind: KIND_MANAGED_AGENT,
      createdAt: 202,
      dTag: "a1",
    }),
  );
  for (let i = 0; i < 3; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  assert.ok(
    !invokedIds.includes("live-cat"),
    "the live catalog head is dropped in degraded mode, not reconciled",
  );
  assert.ok(
    !invokedIds.includes("live-team"),
    "a live team edit is dropped in degraded mode — it could drive a refresh against an unhydrated store",
  );
  assert.ok(
    invokedIds.includes("live-agent"),
    "a live 30177 runtime-policy event still reconciles — degraded sync is not inert",
  );

  mock.reset();
  delete globalThis.window;
});

// Regression guard for Thufir r10-delta finding (degraded-live false unshare).
// A witness-holding device that drops back to degraded-live still receives live
// team/persona edits. Live delivery is newest-first, so a 30176 team edit that
// ADDS a new persona reaches the backend BEFORE that persona's 30175. The
// backend's KIND_TEAM arm unconditionally refreshes the catalog head after a
// save; against the retained witness the new member cannot resolve, so it purges
// the valid witness and queues a false tombstone — the OLD constituents on disk
// don't cover a NEW member. Degraded mode MUST drop the whole catalog dependency
// set (30175/30176 + kind-5 deletions targeting them), not just 30178, so the
// backend never sees the un-hydrated edit. Narrowing the gate back to 30178-only
// dispatches the 30176 to the backend and turns this RED.
test("degraded-live drops a team edit adding a new persona so a witness is not falsely tombstoned", async () => {
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return Promise.resolve();
      },
    },
  };

  // Dense history forces degraded-live on a device that already holds a witness.
  const densePage = [];
  for (let i = 0; i < 500; i += 1)
    densePage.push(
      event({
        id: `dense-${i}`,
        kind: KIND_PERSONA,
        createdAt: 100,
        dTag: `d-${i}`,
      }),
    );
  mock.method(relayClient, "fetchEvents", () => Promise.resolve(densePage));
  let onEvent;
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    onEvent = listener;
    return Promise.resolve(() => Promise.resolve());
  });

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  for (let i = 0; i < 5; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  // Newest-first: the team edit adding P2 arrives before P2's own persona event.
  onEvent(
    event({ id: "team-adds-p2", kind: KIND_TEAM, createdAt: 201, dTag: "t1" }),
  );
  onEvent(
    event({
      id: "new-persona-p2",
      kind: KIND_PERSONA,
      createdAt: 200,
      dTag: "p2",
    }),
  );
  for (let i = 0; i < 3; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  assert.ok(
    !invokedIds.includes("team-adds-p2"),
    "the team edit is dropped — the backend never refreshes against an unresolvable new member, so the witness survives",
  );
  assert.ok(
    !invokedIds.includes("new-persona-p2"),
    "the new persona is dropped too — a lone 30175 cannot complete the dependency set in degraded mode",
  );

  mock.reset();
  delete globalThis.window;
});

// Regression guard for Thufir r11 finding (degraded gate vs Rust router). A
// kind-5 deletion is classified by scanning ALL `a` tags, because Rust's
// `parse_deletion_coordinate` find_maps across every tag and routes the first
// signer-owned dependency coordinate. A valid kind-5 can carry a malformed or
// foreign first `a` tag ahead of an owned 30176 coordinate: reading only the
// first tag returns null and dispatches it, but Rust skips the bad first tag,
// routes the owned 30176, deletes the team, and fires the destructive catalog
// refresh this gate exists to suppress. Degraded mode MUST hold the deletion
// whenever ANY parseable `a` tag names a dependency kind. Narrowing the
// classifier back to the first `a` tag dispatches this deletion and turns RED.
test("degraded-live drops a kind-5 whose owned dependency `a` tag is not first", async () => {
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return Promise.resolve();
      },
    },
  };

  const densePage = [];
  for (let i = 0; i < 500; i += 1)
    densePage.push(
      event({
        id: `dense-${i}`,
        kind: KIND_PERSONA,
        createdAt: 100,
        dTag: `d-${i}`,
      }),
    );
  mock.method(relayClient, "fetchEvents", () => Promise.resolve(densePage));
  let onEvent;
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    onEvent = listener;
    return Promise.resolve(() => Promise.resolve());
  });

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  for (let i = 0; i < 5; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  // Malformed first `a` tag, then an owned 30176 team coordinate — exactly what
  // Rust routes past the bad first tag into a destructive team deletion.
  const deletion = event({
    id: "del-team-second-tag",
    kind: KIND_DELETION,
    createdAt: 201,
    dTag: null,
  });
  deletion.tags = [
    ["a", "not-a-coordinate"],
    ["a", `${KIND_TEAM}:owner-pubkey:t1`],
  ];
  onEvent(deletion);
  for (let i = 0; i < 3; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  assert.ok(
    !invokedIds.includes("del-team-second-tag"),
    "the deletion is dropped — Rust would route its owned 30176 tag into a destructive refresh, so degraded mode must hold it",
  );

  mock.reset();
  delete globalThis.window;
});

// Regression guard for Will r10 P3 finding 2 (backfill rejection stranding live
// sync). A transient history-fetch rejection must not leave the subscription
// permanently unhydrated with `liveBuffer` accumulating forever. The backfill
// MUST retry with bounded backoff; on success the buffer drains and live events
// reconcile. Restoring a log-only `.catch` (no retry, `hydrated` never set)
// leaves the buffered live event unreconciled, turning this RED.
test("startPersonaSync retries a transient backfill rejection so a buffered live event still reconciles", async () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  const invokedIds = [];
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke: (_cmd, args) => {
        invokedIds.push(JSON.parse(args.eventJson).id);
        return Promise.resolve();
      },
    },
  };

  let attempts = 0;
  mock.method(relayClient, "fetchEvents", () => {
    attempts += 1;
    // Fail the first two attempts transiently, then succeed with empty history.
    return attempts < 3
      ? Promise.reject(new Error("relay unreachable"))
      : Promise.resolve([]);
  });
  let onEvent;
  mock.method(relayClient, "subscribeLive", (_filter, listener) => {
    onEvent = listener;
    return Promise.resolve(() => Promise.resolve());
  });

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  await new Promise((resolve) => setImmediate(resolve));

  // A live event arrives while backfill is still failing — it must buffer, not
  // be lost.
  onEvent(
    event({
      id: "live-persona",
      kind: KIND_PERSONA,
      createdAt: 300,
      dTag: "p1",
    }),
  );

  // Drive the bounded backoff timers (500ms, then 1000ms) to the retry that
  // succeeds, flushing the promise chain between ticks.
  for (let i = 0; i < 6; i += 1) {
    mock.timers.tick(2_000);
    await new Promise((resolve) => setImmediate(resolve));
  }

  assert.equal(attempts, 3, "backfill retried until it succeeded");
  assert.ok(
    invokedIds.includes("live-persona"),
    "the buffered live event reconciles after the retry hydrates — not stranded",
  );

  mock.reset();
  mock.timers.reset();
  delete globalThis.window;
});

// `PersonaHistoryDenseBoundaryError` is deterministic (a dense second cannot
// clear on retry), so the pipeline must NOT retry it — it goes straight to
// degraded-live. Guards against a future refactor that lumps it in with
// transient rejections and burns three fetch attempts on an unrecoverable state.
test("startPersonaSync does not retry a dense-boundary error", async () => {
  globalThis.window = {
    __TAURI_INTERNALS__: { invoke: () => Promise.resolve() },
  };
  const densePage = [];
  for (let i = 0; i < 500; i += 1)
    densePage.push(
      event({
        id: `d-${i}`,
        kind: KIND_PERSONA,
        createdAt: 100,
        dTag: `x-${i}`,
      }),
    );
  const fetchCalls = [];
  mock.method(relayClient, "fetchEvents", (filter) => {
    fetchCalls.push(filter);
    return Promise.resolve(densePage);
  });
  mock.method(relayClient, "subscribeLive", () =>
    Promise.resolve(() => Promise.resolve()),
  );

  startPersonaSync("owner-pubkey", "wss://relay.example", () => false);
  for (let i = 0; i < 5; i += 1)
    await new Promise((resolve) => setImmediate(resolve));

  assert.equal(
    fetchCalls.length,
    2,
    "page 1 (full) + page 2 (dense) then abort — no retry attempts",
  );

  mock.reset();
  delete globalThis.window;
});

test("PersonaHistoryDenseBoundaryError names the boundary second", () => {
  const error = new PersonaHistoryDenseBoundaryError(42);
  assert.ok(error instanceof Error);
  assert.equal(error.name, "PersonaHistoryDenseBoundaryError");
  assert.match(error.message, /42/);
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

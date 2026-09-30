import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";
import {
  createActiveAgentTurnsObserverListener,
  getActiveTurnsForAgent,
  resetActiveAgentTurnsStore,
  restoreActiveAgentTurnsForCommunity,
  saveActiveAgentTurnsForCommunity,
  subscribeActiveAgentTurns,
  syncActiveAgentTurnsFromObserver,
} from "./activeAgentTurnsStore.ts";
import {
  createRecentAgentTurnFailuresObserverListener,
  getRecentAgentTurnFailures,
  resetRecentAgentTurnFailuresStore,
  subscribeRecentAgentTurnFailures,
  syncRecentAgentTurnFailuresFromObserver,
} from "./recentAgentTurnFailuresStore.ts";
import {
  resetAgentObserverStore,
  subscribeAgentObserverStore,
  syncAgentObserverEvents,
} from "./observerRelayStore.ts";

const AGENT = "a".repeat(64);
const agents = [{ pubkey: AGENT, status: "running" }];
function frame(seq, kind, turnId, root, payload = {}, timing = "equal") {
  return {
    seq: timing === "restart" && seq === 1 ? 100 : seq,
    timestamp:
      timing === "equal"
        ? "2026-09-21T18:00:00Z"
        : `2026-09-21T18:00:${String(seq).padStart(2, "0")}Z`,
    kind,
    turnId,
    channelId: "channel",
    agentIndex: 0,
    sessionId: "session",
    payload: {
      triggeringEventIds: [root],
      triggeringRootEventId: root,
      ...payload,
    },
  };
}

function subscribeBoth() {
  const unsubscribers = [
    subscribeAgentObserverStore(createActiveAgentTurnsObserverListener(agents)),
    subscribeAgentObserverStore(
      createRecentAgentTurnFailuresObserverListener(agents),
    ),
  ];
  return () => {
    for (const unsubscribe of unsubscribers) unsubscribe();
  };
}
const deliver = (event) => syncAgentObserverEvents(AGENT, [event]);

describe("shutdown ordering through both production observer consumers", () => {
  beforeEach(() => {
    resetAgentObserverStore();
    resetActiveAgentTurnsStore();
    resetRecentAgentTurnFailuresStore();
  });

  for (const timing of ["equal", "restart"]) {
    for (const arrival of ["FIFO", "start buffered", "start delivered"]) {
      for (const outcome of ["success", "action_required"]) {
        for (const sharedRoot of [false, true]) {
          it(`${arrival}, ${outcome}, ${timing}, shared conversation=${sharedRoot}`, () => {
            const bRoot = sharedRoot ? "A" : "B";
            const frames = [
              frame(
                1,
                "turn_error",
                "old-A",
                "A",
                { error: "A failed", disposition: "retrying" },
                timing,
              ),
              frame(2, "turn_started", "retry-A", "A", {}, timing),
              frame(
                3,
                outcome === "success" ? "turn_completed" : "turn_error",
                "retry-A",
                "A",
                { error: "A requires action", disposition: "action_required" },
                timing,
              ),
              frame(4, "turn_started", "B", bRoot, {}, timing),
              frame(
                5,
                "turn_error",
                "B",
                bRoot,
                {
                  error: "B stopped",
                  disposition: "stopped",
                  runtimeExiting: true,
                },
                timing,
              ),
            ];
            let unsubscribe = subscribeBoth();
            const order =
              arrival === "FIFO"
                ? [0, 1, 2, 3, 4]
                : arrival === "start buffered"
                  ? [0, 4, 1, 2, 3]
                  : [0, 1, 4, 2, 3];
            const assertFinal = () => {
              assert.equal(
                getActiveTurnsForAgent(AGENT).length,
                0,
                "no ghost working badge",
              );
              assert.equal(
                getRecentAgentTurnFailures("channel", bRoot)[0]?.error,
                "B stopped",
                "B terminal survives",
              );
              if (!sharedRoot) {
                const a = getRecentAgentTurnFailures("channel", "A");
                assert.equal(
                  a.length,
                  outcome === "success" ? 0 : 1,
                  "A outcome retained",
                );
                if (outcome !== "success") {
                  assert.equal(a[0].disposition, "action_required");
                  assert.equal(a[0].turnId, "retry-A");
                }
              }
            };
            try {
              for (const index of order) deliver(frames[index]);
              assertFinal();
              let notifications = 0;
              const unsubscribeActive = subscribeActiveAgentTurns(
                () => notifications++,
              );
              const unsubscribeFailures = subscribeRecentAgentTurnFailures(
                () => notifications++,
              );
              try {
                frames.forEach(deliver);
                unsubscribe();
                // This is the actual bridge hydration sequence on remount.
                syncActiveAgentTurnsFromObserver(agents);
                syncRecentAgentTurnFailuresFromObserver(agents);
                unsubscribe = subscribeBoth();
                frames.forEach(deliver);
                assertFinal();
                assert.equal(notifications, 0, "replay/remount is a no-op");
              } finally {
                unsubscribeActive();
                unsubscribeFailures();
              }
            } finally {
              unsubscribe();
            }
          });
        }
      }
    }
  }

  it("ordinary terminals still fence older starts, errors and liveness", () => {
    const unsubscribe = subscribeBoth();
    try {
      deliver(
        frame(5, "turn_error", "B", "B", {
          error: "ordinary",
          disposition: "stopped",
        }),
      );
      for (const kind of ["turn_started", "turn_error", "turn_liveness"]) {
        deliver(frame(2, kind, "A", "A", { error: "old error" }));
      }
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
      assert.equal(getRecentAgentTurnFailures("channel", "A").length, 0);
      assert.equal(
        getRecentAgentTurnFailures("channel", "B")[0]?.error,
        "ordinary",
      );
    } finally {
      unsubscribe();
    }
  });

  it("multiple promoted exits do not reopen older turns or replace their failures", () => {
    const unsubscribe = subscribeBoth();
    try {
      const b = frame(
        7,
        "turn_error",
        "B",
        "B",
        { error: "B stopped", disposition: "stopped", runtimeExiting: true },
        "restart",
      );
      const c = frame(
        12,
        "agent_panic",
        "C",
        "C",
        { error: "C stopped", disposition: "stopped", runtimeExiting: true },
        "restart",
      );
      [b, c].forEach(deliver);
      [
        frame(2, "turn_started", "A", "A", {}, "restart"),
        frame(
          3,
          "turn_error",
          "A",
          "A",
          { error: "A requires action", disposition: "action_required" },
          "restart",
        ),
        frame(4, "turn_started", "B", "B", {}, "restart"),
        frame(
          5,
          "turn_error",
          "B",
          "B",
          { error: "B earlier failure", disposition: "retrying" },
          "restart",
        ),
        frame(6, "turn_liveness", "B", "B", {}, "restart"),
        frame(8, "turn_started", "C", "C", {}, "restart"),
        frame(
          9,
          "turn_error",
          "C",
          "C",
          { error: "C earlier failure", disposition: "retrying" },
          "restart",
        ),
        frame(10, "turn_liveness", "C", "C", {}, "restart"),
        b,
        c,
      ].forEach((event) => {
        deliver(event);
        assert.equal(
          getActiveTurnsForAgent(AGENT).length,
          event.kind === "turn_started" && event.turnId === "A" ? 1 : 0,
          `${event.kind} for ${event.turnId} must not revive a stopped turn`,
        );
      });
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
      assert.equal(
        getRecentAgentTurnFailures("channel", "A")[0]?.disposition,
        "action_required",
      );
      assert.equal(
        getRecentAgentTurnFailures("channel", "B")[0]?.error,
        "B stopped",
      );
      assert.equal(
        getRecentAgentTurnFailures("channel", "C")[0]?.error,
        "C stopped",
      );
    } finally {
      unsubscribe();
    }
  });

  it("community restore retains ordinary progress separately from promoted exit", () => {
    const listener = createActiveAgentTurnsObserverListener(agents);
    const update = (...events) => listener({ agentPubkey: AGENT, events });
    const a = frame(2, "turn_started", "A", "A");
    const b = frame(5, "turn_error", "B", "B", { runtimeExiting: true });
    update(a, b);
    saveActiveAgentTurnsForCommunity("promoted-exit");
    resetActiveAgentTurnsStore();
    restoreActiveAgentTurnsForCommunity("promoted-exit");
    assert.equal(getActiveTurnsForAgent(AGENT).length, 1);
    update(
      frame(3, "turn_completed", "A", "A"),
      frame(4, "turn_started", "B", "B"),
    );
    assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
    update(a, b, frame(4, "turn_liveness", "B", "B"));
    assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
  });

  it("a malformed promoted null-turn terminal cannot evict another turn", () => {
    const unsubscribe = subscribeBoth();
    try {
      deliver(frame(1, "turn_started", "A", "A"));
      deliver(frame(5, "agent_panic", null, "B", { runtimeExiting: true }));
      assert.equal(getActiveTurnsForAgent(AGENT).length, 1);
      deliver(frame(2, "turn_completed", "A", "A"));
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
      assert.equal(getRecentAgentTurnFailures("channel", "B").length, 0);
    } finally {
      unsubscribe();
    }
  });

  it("a promoted exit still fences its old frames after terminal tombstone eviction", () => {
    const unsubscribe = subscribeBoth();
    try {
      deliver(
        frame(200, "turn_error", "B", "B", {
          error: "B stopped",
          disposition: "stopped",
          runtimeExiting: true,
        }),
      );
      for (let seq = 1; seq <= 129; seq++) {
        deliver(frame(seq, "turn_completed", `old-${seq}`, `old-${seq}`));
      }
      deliver(frame(150, "turn_started", "B", "B"));
      assert.equal(
        getActiveTurnsForAgent(AGENT).length,
        0,
        "evicted tombstone must not reopen B on start",
      );
      deliver(frame(151, "turn_liveness", "B", "B"));
      assert.equal(
        getActiveTurnsForAgent(AGENT).length,
        0,
        "evicted tombstone must not reopen B on liveness",
      );
      deliver(frame(152, "turn_error", "B", "B", { error: "old B error" }));
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
      assert.equal(
        getRecentAgentTurnFailures("channel", "B")[0]?.error,
        "B stopped",
      );
      // Suppressed B frames still advance ordinary FIFO progress, so an
      // unrelated older replay cannot create a failure or working badge.
      deliver(frame(140, "turn_started", "replay", "replay"));
      deliver(
        frame(141, "turn_error", "replay", "replay", { error: "replayed" }),
      );
      assert.equal(getActiveTurnsForAgent(AGENT).length, 0);
      assert.equal(getRecentAgentTurnFailures("channel", "replay").length, 0);
    } finally {
      unsubscribe();
    }
  });

  it("older buffered failures cannot evict the newest promoted failure", () => {
    const unsubscribe = subscribeBoth();
    try {
      deliver(
        frame(200, "turn_error", "B", "B", {
          error: "B stopped",
          disposition: "stopped",
          runtimeExiting: true,
        }),
      );
      for (let seq = 1; seq <= 25; seq++) {
        deliver(
          frame(seq, "turn_error", `old-${seq}`, `old-${seq}`, {
            error: "old failure",
          }),
        );
      }
      assert.equal(
        getRecentAgentTurnFailures("channel", "B")[0]?.error,
        "B stopped",
      );
      assert.equal(getRecentAgentTurnFailures("channel").length, 20);
      assert.equal(getRecentAgentTurnFailures("channel", "old-1").length, 0);
      assert.equal(getRecentAgentTurnFailures("channel", "old-25").length, 1);
    } finally {
      unsubscribe();
    }
  });
});

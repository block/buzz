import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { beforeEach, describe, it } from "node:test";

import {
  createRecentAgentTurnFailuresObserverListener,
  getRecentAgentTurnFailures,
  resetRecentAgentTurnFailuresStore,
  subscribeRecentAgentTurnFailures,
  syncRecentAgentTurnFailuresFromEvents,
  syncRecentAgentTurnFailuresFromObserver,
} from "./recentAgentTurnFailuresStore.ts";
import {
  resetAgentObserverStore,
  subscribeAgentObserverStore,
  syncAgentObserverEvents,
} from "./observerRelayStore.ts";

const AGENT = "a".repeat(64);
const TRIGGER = "1".repeat(64);
const ROOT = "2".repeat(64);
const TOP_LEVEL = "3".repeat(64);

function event(overrides = {}) {
  return {
    seq: 1,
    timestamp: "2026-09-09T17:49:05Z",
    kind: "turn_started",
    agentIndex: 0,
    channelId: "channel-1",
    sessionId: "session-1",
    turnId: "turn-1",
    payload: {
      triggeringEventIds: [TRIGGER],
      triggeringRootEventId: ROOT,
      triggeringParentEventId: TRIGGER,
    },
    ...overrides,
  };
}

describe("recentAgentTurnFailuresStore", () => {
  beforeEach(resetRecentAgentTurnFailuresStore);

  it("retains a friendly retrying failure in its originating conversation", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event(),
      event({
        seq: 2,
        timestamp: "2026-09-09T17:49:09Z",
        kind: "turn_error",
        payload: {
          error: "Agent reported error (code -32603): Internal error",
          code: -32603,
          disposition: "retrying",
          attempt: 1,
          triggeringEventIds: [TRIGGER],
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
    ]);

    const failures = getRecentAgentTurnFailures("channel-1", ROOT);
    assert.equal(failures.length, 1);
    assert.equal(failures[0].disposition, "retrying");
    assert.equal(failures[0].attempt, 1);
    assert.equal(failures[0].rootEventId, ROOT);
    assert.match(failures[0].error, /upgrade the adapter/);
    assert.equal(getRecentAgentTurnFailures("channel-1", TRIGGER).length, 0);
  });

  it("updates one conversation failure instead of accumulating retry spam", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event(),
      event({
        seq: 2,
        kind: "turn_error",
        payload: {
          error: "Internal error",
          code: -32603,
          disposition: "retrying",
          attempt: 1,
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
      event({
        seq: 3,
        timestamp: "2026-09-09T17:50:09Z",
        kind: "turn_error",
        turnId: "turn-2",
        payload: {
          error: "Internal error",
          code: -32603,
          disposition: "dead_lettered",
          attempt: 11,
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
    ]);

    const failures = getRecentAgentTurnFailures("channel-1", ROOT);
    assert.equal(failures.length, 1);
    assert.equal(failures[0].disposition, "dead_lettered");
    assert.equal(failures[0].attempt, 11);
    assert.equal(failures[0].turnId, "turn-2");
  });

  it("clears the retained failure when the conversation starts a new turn", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event(),
      event({
        seq: 2,
        kind: "turn_error",
        payload: {
          error: "Internal error",
          disposition: "retrying",
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
    ]);
    assert.equal(getRecentAgentTurnFailures("channel-1", ROOT).length, 1);

    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event({
        seq: 3,
        timestamp: "2026-09-09T17:50:09Z",
        turnId: "turn-2",
      }),
    ]);
    assert.equal(getRecentAgentTurnFailures("channel-1", ROOT).length, 0);
  });

  it("keeps threaded failures out of the main composer scope", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event({
        seq: 2,
        kind: "turn_error",
        payload: {
          error: "thread failure",
          triggeringEventIds: [TRIGGER],
          triggeringRootEventId: ROOT,
          triggeringParentEventId: TRIGGER,
        },
      }),
      event({
        seq: 3,
        timestamp: "2026-09-09T17:50:09Z",
        kind: "turn_error",
        turnId: "turn-2",
        payload: {
          error: "top-level failure",
          triggeringEventIds: [TOP_LEVEL],
          triggeringRootEventId: TOP_LEVEL,
          triggeringParentEventId: TOP_LEVEL,
        },
      }),
    ]);

    assert.equal(
      getRecentAgentTurnFailures("channel-1")[0]?.error,
      "top-level failure",
    );
    assert.equal(
      getRecentAgentTurnFailures("channel-1", ROOT)[0]?.error,
      "thread failure",
    );
  });

  it("processes only observer updates for an active agent", () => {
    const listener = createRecentAgentTurnFailuresObserverListener([
      { pubkey: AGENT, status: "running" },
      { pubkey: "b".repeat(64), status: "stopped" },
    ]);
    const failureEvent = event({
      seq: 2,
      kind: "turn_error",
      payload: {
        error: "live failure",
        triggeringRootEventId: ROOT,
        triggeringParentEventId: TRIGGER,
      },
    });

    listener({ agentPubkey: "b".repeat(64), events: [failureEvent] });
    assert.equal(getRecentAgentTurnFailures("channel-1", ROOT).length, 0);

    listener({ agentPubkey: AGENT, events: [event(), failureEvent] });
    assert.equal(
      getRecentAgentTurnFailures("channel-1", ROOT)[0]?.error,
      "live failure",
    );
  });

  it("keeps actual legacy source-and-ID-only context unknown and visible", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event({ payload: { source: "mention", triggeringEventIds: [TRIGGER] } }),
      event({
        seq: 2,
        kind: "turn_error",
        payload: { error: "legacy failure" },
      }),
    ]);

    const [failure] = getRecentAgentTurnFailures("channel-1");
    assert.equal(failure.error, "legacy failure");
    assert.equal(failure.rootEventId, null);
    assert.equal(failure.parentEventId, null);
    assert.equal(failure.disposition, "unknown");
    assert.equal(getRecentAgentTurnFailures("channel-1", ROOT).length, 0);
    assert.equal(getRecentAgentTurnFailures("channel-1", TRIGGER).length, 0);
  });
});

describe("retry batch coverage through observer listener", () => {
  beforeEach(resetRecentAgentTurnFailuresStore);

  it("tracks the Rust-emitted full Steer retry lifecycle through exhaustion", () => {
    // The production run_prompt_task -> handle_prompt_result regression in
    // buzz-acp compares its emitted correlation and outcomes to this SAME
    // fixture. Only transport envelope fields are supplied here.
    const lifecycle = JSON.parse(
      readFileSync(
        new URL(
          "../../../../crates/buzz-acp/tests/fixtures/steer-retry-observer.json",
          import.meta.url,
        ),
        "utf8",
      ),
    );
    const listener = createRecentAgentTurnFailuresObserverListener([
      { pubkey: AGENT, status: "running" },
    ]);
    listener({
      agentPubkey: AGENT,
      events: [
        event({
          kind: "turn_error",
          turnId: "unrelated-turn",
          payload: {
            error: "unrelated failure",
            disposition: "stopped",
            triggeringEventIds: ["unrelated-request"],
            triggeringRootEventId: "unrelated-root",
          },
        }),
      ],
    });

    assert.deepEqual(
      lifecycle.map(({ kind }) => kind),
      [
        "turn_started",
        "turn_error",
        "turn_started",
        "turn_error",
        "turn_started",
        "turn_error",
      ],
    );
    for (const [index, emitted] of lifecycle.entries()) {
      listener({
        agentPubkey: AGENT,
        events: [event({ ...emitted, seq: index + 2 })],
      });
      const failures = getRecentAgentTurnFailures("channel-1", "thread-root");
      if (emitted.kind === "turn_started") {
        assert.equal(
          failures.length,
          0,
          `full retry ${index} clears its prior failure`,
        );
      } else {
        assert.equal(failures.length, 1);
        assert.deepEqual(failures[0].triggeringEventIds, [
          "request-a",
          "request-b",
        ]);
        assert.equal(failures[0].parentEventId, "thread-parent");
        assert.equal(failures[0].turnId, emitted.turnId);
        assert.equal(failures[0].disposition, emitted.payload.disposition);
        assert.equal(failures[0].attempt, emitted.payload.attempt);
      }
      assert.equal(
        getRecentAgentTurnFailures("channel-1", "unrelated-root").length,
        1,
      );
    }
    const [terminal] = getRecentAgentTurnFailures("channel-1", "thread-root");
    assert.equal(terminal.disposition, "dead_lettered");
    assert.equal(terminal.attempt, 11);
  });

  it("clears A when retrying [A,B] anchored at B, preserving partial and unrelated failures", () => {
    const listener = createRecentAgentTurnFailuresObserverListener([
      { pubkey: AGENT, status: "running" },
    ]);
    const failed = (seq, ids, root, overrides = {}) =>
      event({
        seq,
        kind: "turn_error",
        turnId: `failed-${seq}`,
        payload: {
          error: "failed",
          disposition: "retrying",
          triggeringEventIds: ids,
          triggeringRootEventId: root,
          ...overrides,
        },
      });
    listener({
      agentPubkey: AGENT,
      events: [
        failed(1, ["A"], "A"),
        failed(2, ["A", "C"], "C"),
        failed(3, ["D"], "D"),
        { ...failed(4, ["A"], "A"), channelId: "other-channel" },
        event({
          seq: 5,
          turnId: "retry-ab",
          payload: {
            triggeringEventIds: ["A", "B"],
            triggeringRootEventId: "B",
            triggeringParentEventId: "B",
          },
        }),
      ],
    });
    assert.equal(getRecentAgentTurnFailures("channel-1", "A").length, 0);
    assert.equal(getRecentAgentTurnFailures("channel-1", "C").length, 1);
    assert.equal(getRecentAgentTurnFailures("channel-1", "D").length, 1);
    assert.equal(getRecentAgentTurnFailures("other-channel", "A").length, 1);
  });

  it("preserves full started batch when an error only reports its root", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event({
        payload: { triggeringEventIds: ["A", "B"], triggeringRootEventId: "B" },
      }),
      event({
        seq: 2,
        kind: "turn_error",
        payload: { error: "failed", triggeringRootEventId: "B" },
      }),
      event({
        seq: 3,
        turnId: "partial-retry",
        payload: { triggeringEventIds: ["B"], triggeringRootEventId: "B" },
      }),
    ]);
    assert.deepEqual(
      getRecentAgentTurnFailures("channel-1", "B")[0].triggeringEventIds,
      ["A", "B"],
    );
  });

  it("accepts panic context without a preceding start and preserves reported recovery", () => {
    syncRecentAgentTurnFailuresFromEvents(AGENT, [
      event({
        kind: "agent_panic",
        payload: {
          error: "panic",
          triggeringEventIds: [TRIGGER],
          triggeringRootEventId: ROOT,
          disposition: "retrying",
          respawnScheduled: false,
          attempt: 2,
        },
      }),
    ]);
    const [failure] = getRecentAgentTurnFailures("channel-1", ROOT);
    assert.equal(failure.disposition, "retrying");
    assert.equal(failure.respawnScheduled, false);
    assert.equal(failure.attempt, 2);
  });

  it("keeps a shutdown-promoted terminal when older telemetry arrives later", () => {
    const listener = createRecentAgentTurnFailuresObserverListener([
      { pubkey: AGENT, status: "running" },
    ]);
    const terminal = event({
      seq: 20,
      kind: "turn_error",
      payload: {
        error: "final worker exited",
        disposition: "stopped",
        respawnScheduled: false,
        runtimeExiting: true,
        triggeringEventIds: [TRIGGER],
        triggeringRootEventId: ROOT,
        triggeringParentEventId: TRIGGER,
      },
    });
    listener({ agentPubkey: AGENT, events: [terminal] });
    listener({
      agentPubkey: AGENT,
      events: [
        event({ seq: 10 }),
        {
          ...terminal,
          seq: 11,
          payload: { ...terminal.payload, disposition: "retrying" },
        },
        event({ seq: 12, kind: "turn_completed" }),
        { ...terminal, seq: 1, channelId: "other-channel" },
      ],
    });
    const [failure] = getRecentAgentTurnFailures("channel-1", ROOT);
    assert.equal(failure.disposition, "stopped");
    assert.equal(failure.respawnScheduled, false);
    assert.equal(failure.error, "final worker exited");
    assert.equal(getRecentAgentTurnFailures("other-channel", ROOT).length, 1);
  });
});

describe("late recovery after shutdown terminal promotion", () => {
  beforeEach(() => {
    resetAgentObserverStore();
    resetRecentAgentTurnFailuresStore();
  });

  const agents = [{ pubkey: AGENT, status: "running" }];
  const context = (ids, root = ids.at(-1)) => ({
    triggeringEventIds: ids,
    triggeringRootEventId: root,
  });

  for (const timing of ["equal timestamp", "timestamp before sequence"]) {
    for (const order of ["FIFO", "terminal promoted"]) {
      it(`clears recovered A and keeps stopped B: ${order}, ${timing}`, () => {
        const frames = [
          event({
            kind: "turn_error",
            payload: {
              ...context(["A"]),
              error: "A failed",
              disposition: "retrying",
            },
          }),
          event({ turnId: "retry-A", payload: context(["A"]) }),
          event({ kind: "turn_completed", turnId: "retry-A", payload: {} }),
          event({ turnId: "turn-B", payload: context(["B"]) }),
          event({
            kind: "turn_error",
            turnId: "turn-B",
            payload: {
              ...context(["B"]),
              error: "B stopped",
              disposition: "stopped",
              runtimeExiting: true,
              respawnScheduled: false,
            },
          }),
        ].map((frame, index) => ({
          ...frame,
          // The second mode also covers a seq reset after A's failure.
          seq:
            timing === "equal timestamp"
              ? index + 1
              : index === 0
                ? 100
                : index,
          timestamp:
            timing === "equal timestamp"
              ? "2026-09-09T17:49:05Z"
              : `2026-09-09T17:49:0${index}Z`,
        }));
        let unsubscribe = subscribeAgentObserverStore(
          createRecentAgentTurnFailuresObserverListener(agents),
        );
        const deliver = (frame) => syncAgentObserverEvents(AGENT, [frame]);
        const assertRecovered = () => {
          assert.equal(
            getRecentAgentTurnFailures("channel-1", "A").length,
            0,
            "A recovered",
          );
          const [failure] = getRecentAgentTurnFailures("channel-1", "B");
          assert.equal(failure?.error, "B stopped");
          assert.equal(failure?.disposition, "stopped");
          assert.equal(failure?.respawnScheduled, false);
          assert.equal(getRecentAgentTurnFailures("channel-1").length, 1);
        };
        try {
          for (const index of order === "FIFO"
            ? [0, 1, 2, 3, 4]
            : [0, 4, 1, 2, 3]) {
            deliver(frames[index]);
          }
          assertRecovered();

          let notifications = 0;
          const unsubscribeFailures = subscribeRecentAgentTurnFailures(
            () => notifications++,
          );
          try {
            // Transport duplicates, a replayed buffer, and the actual bridge's
            // snapshot hydration on remount must not revive A or clear B.
            frames.forEach(deliver);
            syncRecentAgentTurnFailuresFromEvents(AGENT, frames);
            unsubscribe();
            syncRecentAgentTurnFailuresFromObserver(agents);
            unsubscribe = subscribeAgentObserverStore(
              createRecentAgentTurnFailuresObserverListener(agents),
            );
            frames.forEach(deliver);
            assertRecovered();
            assert.equal(
              notifications,
              0,
              "duplicate/replay/remount is idempotent",
            );
          } finally {
            unsubscribeFailures();
          }
        } finally {
          unsubscribe();
        }
      });
    }
  }

  it("late retry clears only fully covered, strictly older failures", () => {
    const listener = createRecentAgentTurnFailuresObserverListener(agents);
    const failed = (seq, ids, root, extra = {}) =>
      event({
        seq,
        kind: "turn_error",
        turnId: `failure-${root}`,
        payload: { ...context(ids, root), error: root, disposition: "stopped" },
        ...extra,
      });
    listener({
      agentPubkey: AGENT,
      events: [
        failed(1, ["A"], "old"),
        failed(2, ["A", "C"], "partial"),
        failed(3, ["D"], "unrelated"),
        failed(4, [], "legacy"),
        ...[
          failed(5, ["A"], "equal"),
          failed(6, ["A"], "newer"),
          failed(7, ["A"], "same-conversation"),
          failed(8, [], "legacy-newer"),
        ].map((frame) => ({
          ...frame,
          payload: { ...frame.payload, runtimeExiting: true },
        })),
        failed(9, ["A"], "other-channel", { channelId: "channel-2" }),
      ],
    });
    syncRecentAgentTurnFailuresFromEvents("b".repeat(64), [
      failed(1, ["A"], "other-agent"),
    ]);
    listener({
      agentPubkey: AGENT,
      events: [
        event({
          seq: 5,
          turnId: "late-retry",
          payload: context(["A"], "same-conversation"),
        }),
      ],
    });
    assert.equal(getRecentAgentTurnFailures("channel-1", "old").length, 0);
    for (const root of [
      "partial",
      "unrelated",
      "legacy",
      "equal",
      "newer",
      "same-conversation",
      "legacy-newer",
    ]) {
      assert.equal(
        getRecentAgentTurnFailures("channel-1", root).length,
        1,
        `${root} survives`,
      );
    }
    listener({
      agentPubkey: AGENT,
      events: [event({ seq: 6, payload: context([], "legacy") })],
    });
    listener({
      agentPubkey: AGENT,
      events: [event({ seq: 7, payload: context([], "legacy-newer") })],
    });
    assert.equal(getRecentAgentTurnFailures("channel-1", "legacy").length, 0);
    assert.equal(
      getRecentAgentTurnFailures("channel-1", "legacy-newer").length,
      1,
    );
    assert.equal(
      getRecentAgentTurnFailures("channel-2", "other-channel").length,
      1,
    );
    assert.equal(
      getRecentAgentTurnFailures("channel-1", "other-agent").length,
      1,
    );
  });

  it("timestamp ordering protects a newer failure despite its lower sequence", () => {
    const listener = createRecentAgentTurnFailuresObserverListener(agents);
    const terminal = event({
      seq: 1,
      timestamp: "2026-09-09T17:50:00Z",
      kind: "turn_error",
      payload: {
        ...context(["A"]),
        error: "new runtime stopped",
        disposition: "stopped",
      },
    });
    listener({ agentPubkey: AGENT, events: [terminal] });
    listener({
      agentPubkey: AGENT,
      events: [
        event({ seq: 100, payload: context(["A"]) }),
        {
          ...terminal,
          seq: 99,
          timestamp: "2026-09-09T17:49:05Z",
          payload: { ...terminal.payload, disposition: "retrying" },
        },
      ],
    });
    assert.equal(
      getRecentAgentTurnFailures("channel-1", "A")[0]?.disposition,
      "stopped",
    );
  });
});

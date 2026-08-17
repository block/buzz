import assert from "node:assert/strict";
import test from "node:test";

import {
  applyMentionAck,
  clearPendingMention,
  getMentionProblem,
  getPendingMention,
  getProblemEventIds,
  MENTION_ACK_TIMEOUT_MS,
  registerPendingMention,
  resetPendingMentionAckStore,
  subscribePendingMentionAcks,
} from "./pendingMentionAckStore.ts";

const AGENT_A = "a".repeat(64);
const AGENT_B = "b".repeat(64);
const STRANGER = "c".repeat(64);

/** Run `fn` with the store's timeout collapsed to something test-sized. */
function withFakeClock(fn) {
  const realSetTimeout = globalThis.setTimeout;
  const realClearTimeout = globalThis.clearTimeout;
  const pending = new Map();
  let nextId = 1;

  globalThis.setTimeout = (cb, ms) => {
    const id = nextId++;
    pending.set(id, { cb, ms });
    return id;
  };
  globalThis.clearTimeout = (id) => {
    pending.delete(id);
  };

  /** Fire every scheduled timer, as if the wall clock advanced past them. */
  const advance = () => {
    for (const [id, { cb }] of [...pending]) {
      pending.delete(id);
      cb();
    }
  };

  try {
    return fn(advance, pending);
  } finally {
    globalThis.setTimeout = realSetTimeout;
    globalThis.clearTimeout = realClearTimeout;
    resetPendingMentionAckStore();
  }
}

test("pendingMentionAck_noProblemWhileWaiting", () => {
  withFakeClock(() => {
    registerPendingMention("evt1", "chan", [AGENT_A]);
    // Before the timeout, silence is normal — the agent may simply be thinking.
    assert.equal(getMentionProblem("evt1"), null);
  });
});

test("pendingMentionAck_acceptedNeverSurfaces", () => {
  withFakeClock((advance) => {
    registerPendingMention("evt2", "chan", [AGENT_A]);
    applyMentionAck("evt2", AGENT_A, "accepted", null);
    advance();
    // An accepted mention is on its way; showing anything would be noise.
    assert.equal(getMentionProblem("evt2"), null);
  });
});

test("pendingMentionAck_declinedSurfacesWithReason", () => {
  withFakeClock(() => {
    registerPendingMention("evt3", "chan", [AGENT_A]);
    applyMentionAck("evt3", AGENT_A, "declined", "sender-not-allowed");
    const problem = getMentionProblem("evt3");
    assert.ok(problem, "a decline must surface");
    assert.deepEqual(problem.declined, [
      { pubkey: AGENT_A, reason: "sender-not-allowed" },
    ]);
    assert.deepEqual(problem.silent, []);
  });
});

test("pendingMentionAck_declineSettlesImmediatelyWithoutWaiting", () => {
  withFakeClock((_advance, pending) => {
    registerPendingMention("evt4", "chan", [AGENT_A]);
    applyMentionAck("evt4", AGENT_A, "declined", "no-matching-rule");
    // Every mentioned agent has reported, so the timer is cancelled rather
    // than left to fire 30s later.
    assert.equal(pending.size, 0, "timer must be cleared once all agents ack");
    assert.ok(getMentionProblem("evt4"));
  });
});

test("pendingMentionAck_timeoutMarksSilent", () => {
  withFakeClock((advance) => {
    registerPendingMention("evt5", "chan", [AGENT_A]);
    advance();
    const problem = getMentionProblem("evt5");
    assert.ok(problem, "an unacknowledged mention must surface after timeout");
    assert.deepEqual(problem.silent, [AGENT_A]);
    assert.deepEqual(problem.declined, []);
  });
});

test("pendingMentionAck_partialAcceptSuppressesSilentSibling", () => {
  withFakeClock((advance) => {
    registerPendingMention("evt6", "chan", [AGENT_A, AGENT_B]);
    applyMentionAck("evt6", AGENT_A, "accepted", null);
    advance();
    // One agent took it. The team mention is answered, so a warning about the
    // other would be actively misleading.
    assert.equal(getMentionProblem("evt6"), null);
  });
});

test("pendingMentionAck_allDeclinedSurfacesEveryAgent", () => {
  withFakeClock(() => {
    registerPendingMention("evt7", "chan", [AGENT_A, AGENT_B]);
    applyMentionAck("evt7", AGENT_A, "declined", "busy");
    applyMentionAck("evt7", AGENT_B, "declined", "sender-not-allowed");
    const problem = getMentionProblem("evt7");
    assert.equal(problem.declined.length, 2);
  });
});

test("pendingMentionAck_ignoresAckFromUnmentionedPubkey", () => {
  withFakeClock((advance) => {
    registerPendingMention("evt8", "chan", [AGENT_A]);
    // Any community member can publish a well-formed 44102 — the relay cannot
    // check agent-ness. An ack from a pubkey the message never mentioned must
    // not be able to suppress the real warning.
    applyMentionAck("evt8", STRANGER, "accepted", null);
    advance();
    const problem = getMentionProblem("evt8");
    assert.ok(problem, "forged ack must not suppress the warning");
    assert.deepEqual(problem.silent, [AGENT_A]);
  });
});

test("pendingMentionAck_replyClearsTracking", () => {
  withFakeClock((advance) => {
    registerPendingMention("evt9", "chan", [AGENT_A]);
    clearPendingMention("evt9");
    advance();
    // An actual reply outranks the ack path entirely.
    assert.equal(getMentionProblem("evt9"), null);
  });
});

test("pendingMentionAck_humanOnlyMentionIsNotTracked", () => {
  withFakeClock((advance) => {
    registerPendingMention("evt10", "chan", []);
    advance();
    // Mentioning a colleague must never produce "they did not respond".
    assert.equal(getMentionProblem("evt10"), null);
  });
});

test("pendingMentionAck_snapshotIsReferenceStable", () => {
  withFakeClock((advance) => {
    registerPendingMention("evt11", "chan", [AGENT_A]);
    advance();
    // useSyncExternalStore compares by identity — a getter that rebuilds the
    // object each call renders forever. See AGENTS.md gotcha 7.
    assert.equal(getMentionProblem("evt11"), getMentionProblem("evt11"));
  });
});

test("pendingMentionAck_doesNotNotifyWhenNothingChanged", () => {
  withFakeClock(() => {
    let notifications = 0;
    const unsubscribe = subscribePendingMentionAcks(() => {
      notifications += 1;
    });
    registerPendingMention("evt12", "chan", [AGENT_A]);
    // Registering is invisible: the waiting state has no UI, so it must not
    // wake every subscribed row.
    assert.equal(notifications, 0);
    unsubscribe();
  });
});

test("pendingMentionAck_resetClearsEntriesButKeepsListeners", () => {
  withFakeClock(() => {
    let notifications = 0;
    const unsubscribe = subscribePendingMentionAcks(() => {
      notifications += 1;
    });
    registerPendingMention("evt13", "chan", [AGENT_A]);
    applyMentionAck("evt13", AGENT_A, "declined", "busy");
    assert.ok(getMentionProblem("evt13"));

    resetPendingMentionAckStore();
    assert.equal(getMentionProblem("evt13"), null);
    // The listener must survive: components that outlive a community switch
    // would otherwise be subscribed to a store that can never notify them.
    const before = notifications;
    registerPendingMention("evt14", "chan", [AGENT_A]);
    applyMentionAck("evt14", AGENT_A, "declined", "busy");
    assert.ok(notifications > before, "listeners must still fire after reset");
    unsubscribe();
  });
});

test("pendingMentionAck_problemIdsStableAcrossInterleavedChannels", () => {
  withFakeClock((advance) => {
    registerPendingMention("evtA", "chan1", [AGENT_A]);
    registerPendingMention("evtB", "chan2", [AGENT_B]);
    advance();

    // A single-slot cache keyed on the last channel asked about returns a fresh
    // array whenever two channels interleave — which under useSyncExternalStore
    // is the "getSnapshot should be cached" infinite render loop.
    const first = getProblemEventIds("chan1");
    getProblemEventIds("chan2");
    assert.equal(getProblemEventIds("chan1"), first);
  });
});

test("pendingMentionAck_problemIdsInvalidateWhenAProblemChanges", () => {
  withFakeClock((advance) => {
    registerPendingMention("evtC", "chan3", [AGENT_A]);
    assert.deepEqual(getProblemEventIds("chan3"), []);
    advance();
    assert.deepEqual(getProblemEventIds("chan3"), ["evtC"]);
    clearPendingMention("evtC");
    assert.deepEqual(getProblemEventIds("chan3"), []);
  });
});

test("pendingMentionAck_healthyEntryIsReclaimed", () => {
  withFakeClock(() => {
    registerPendingMention("evtD", "chan4", [AGENT_A]);
    applyMentionAck("evtD", AGENT_A, "accepted", null);
    // Settled with nothing to show — it can never produce a problem again, so
    // it must not be carried for the rest of the session and re-scanned on
    // every timeline render.
    assert.equal(getPendingMention("evtD"), undefined);
  });
});

test("pendingMentionAck_timeoutConstantIsGenerousEnoughForAgentStartup", () => {
  // A managed agent is launched as part of the send flow; a premature warning
  // on an agent that was merely booting is worse than a late one.
  assert.ok(MENTION_ACK_TIMEOUT_MS >= 20_000);
});

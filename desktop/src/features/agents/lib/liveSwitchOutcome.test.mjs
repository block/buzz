import assert from "node:assert/strict";
import test from "node:test";

import { awaitLiveSwitchOutcome } from "./liveSwitchOutcome.ts";

const REQUEST_ID = "req-abc";
const CH_A = "channel-a";
const CH_B = "channel-b";

function frame(status, overrides = {}) {
  return {
    type: "switch_model",
    status,
    requestId: REQUEST_ID,
    channelId: CH_A,
    ...overrides,
  };
}

/**
 * A controllable test harness mirroring the real wiring: a single-listener
 * pub/sub whose unsubscribe genuinely detaches (so post-unsubscribe pushes are
 * no-ops, matching `observerRelayStore`), a manual timeout, and a deferred
 * `sendSwitches` the test resolves explicitly.
 */
function harness(channelIds, requestId = REQUEST_ID) {
  let listener = null;
  let timeoutCb = null;
  let unsubscribeCalls = 0;
  let cancelTimeoutCalls = 0;
  let sendResolve;
  const sendStarted = new Promise((resolve) => {
    sendResolve = resolve;
  });

  const outcome = awaitLiveSwitchOutcome({
    requestId,
    channelIds,
    subscribe: (fn) => {
      listener = fn;
      return () => {
        unsubscribeCalls += 1;
        listener = null;
      };
    },
    sendSwitches: () => {
      sendResolve();
      return Promise.resolve();
    },
    scheduleTimeout: (cb) => {
      timeoutCb = cb;
      return () => {
        cancelTimeoutCalls += 1;
      };
    },
  });

  return {
    outcome,
    sendStarted,
    push: (f) => listener?.(f),
    fireTimeout: () => timeoutCb?.(),
    get unsubscribeCalls() {
      return unsubscribeCalls;
    },
    get cancelTimeoutCalls() {
      return cancelTimeoutCalls;
    },
  };
}

const drainMicrotasks = async () => {
  for (let i = 0; i < 5; i++) {
    await Promise.resolve();
  }
};

test("ambiguous sibling sessions reject the pick even after another channel switched", async () => {
  const h = harness([CH_A, CH_B]);
  h.push(frame("switched"));
  h.push(frame("ambiguous_target", { channelId: CH_B }));
  assert.equal(await h.outcome, "ambiguous");
  assert.equal(h.cancelTimeoutCalls, 1);
  assert.equal(h.unsubscribeCalls, 1);
});

test("stale or foreign ambiguity does not reject a live model pick", async () => {
  const h = harness([CH_A]);
  h.push(frame("ambiguous_target", { requestId: "old-pick" }));
  h.push(frame("ambiguous_target", { channelId: CH_B }));
  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome fast sent on one channel does not mask a later unsupported on another", async () => {
  const h = harness([CH_A, CH_B]);
  // Channel A acks fast as `sent`; a first-ack-resolves impl would settle "ok"
  // here. The fail-fast contract must keep waiting and then reject on B.
  h.push(frame("sent"));
  h.push(frame("unsupported_model", { channelId: CH_B }));
  assert.equal(await h.outcome, "unsupported");
});

test("awaitLiveSwitchOutcome resolves ok only after every distinct channel acks", async () => {
  const h = harness([CH_A, CH_B]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  // Terminal success for channel A alone must not settle a two-channel pick.
  h.push(frame("switched", { channelId: CH_A }));
  await drainMicrotasks();
  assert.equal(settled, false, "must not resolve before every channel acks");

  h.push(frame("switched", { channelId: CH_B }));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome settles not_delivered immediately on a turn_ending frame without waiting for other channels or the timeout", async () => {
  // `turn_ending` means the control oneshot was already consumed (a prior
  // cancel is ending the turn) — the switch can't land and nothing applies
  // later. It must fail-fast to "not_delivered", NOT count as a positive
  // terminal. Three channels prove it never traverses the success-count path.
  const h = harness([CH_A, CH_B, "channel-c"]);
  h.push(frame("turn_ending"));
  assert.equal(await h.outcome, "not_delivered");
  assert.equal(h.cancelTimeoutCalls, 1, "timeout cancelled, not awaited");
  assert.equal(h.unsubscribeCalls, 1);

  // A later positive frame must not re-resolve or re-unsubscribe.
  h.push(frame("switched"));
  assert.equal(h.unsubscribeCalls, 1, "no double-unsubscribe on a late frame");
});

test("awaitLiveSwitchOutcome settles not_delivered immediately on a no_active_turn frame", async () => {
  // `no_active_turn` means neither an in-flight task nor an idle session-owning
  // agent existed by the time the harness received the switch (a stale
  // `activeTurns` snapshot). Nothing was applied and nothing rides a later
  // session — fail-fast to "not_delivered", never a false "ok".
  const h = harness([CH_A, CH_B]);
  h.push(frame("no_active_turn"));
  assert.equal(await h.outcome, "not_delivered");
  assert.equal(h.cancelTimeoutCalls, 1, "timeout cancelled, not awaited");
  assert.equal(h.unsubscribeCalls, 1);
});

test("awaitLiveSwitchOutcome ignores an unknown future status and settles via a real switched terminal", async () => {
  // A status the picker doesn't know (a newer harness) must be inert — never
  // default-counted as success. The pick stays open until a real `switched`
  // terminal (or the timeout) settles it.
  const h = harness([CH_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("some_future_status"));
  await drainMicrotasks();
  assert.equal(settled, false, "an unknown status must not settle the pick");

  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome rejects on unsupported immediately and unsubscribes exactly once", async () => {
  const h = harness([CH_A, CH_B]);
  h.push(frame("unsupported_model"));
  assert.equal(await h.outcome, "unsupported");
  assert.equal(h.unsubscribeCalls, 1);
  assert.equal(h.cancelTimeoutCalls, 1);

  // A second rejection arriving after the first must not re-resolve or
  // re-unsubscribe — the listener is already detached.
  h.push(frame("unsupported_model"));
  assert.equal(h.unsubscribeCalls, 1, "no double-unsubscribe on a late frame");
});

test("awaitLiveSwitchOutcome settles failed immediately on an adapter-refused frame without waiting for other channels or the timeout", async () => {
  const h = harness([CH_A, CH_B, "channel-c"]);
  // Three channels, so a success-path impl would need three acks. A single
  // `failure` frame must fail-fast to "failed" (not "unsupported", not "ok")
  // before the other two channels reply — proving it never traverses the
  // success-count path.
  h.push(frame("failure"));
  assert.equal(await h.outcome, "failed");
  // The timeout was cancelled (no 8s wait) and the listener detached exactly
  // once — the frame settled synchronously, not via the fallback.
  assert.equal(h.cancelTimeoutCalls, 1, "timeout cancelled, not awaited");
  assert.equal(h.unsubscribeCalls, 1);

  // A later frame must not re-resolve or re-unsubscribe.
  h.push(frame("switched"));
  assert.equal(h.unsubscribeCalls, 1, "no double-unsubscribe on a late frame");
});

test("awaitLiveSwitchOutcome stays unsettled after a provisional sent, then settles failed when the adapter rejection arrives", async () => {
  // The real busy-path producer order: the harness acks `sent` immediately
  // (the switch was delivered to the in-flight turn), then — after the requeued
  // session consults the adapter — emits `failure` seconds later. With one
  // active channel a first-ack-resolves impl would settle "ok" on `sent` and
  // detach before `failure` arrives; this regresses that.
  const h = harness([CH_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("sent"));
  await drainMicrotasks();
  assert.equal(settled, false, "provisional `sent` must not resolve the pick");
  assert.equal(h.unsubscribeCalls, 0, "subscription stays alive after `sent`");
  assert.equal(h.cancelTimeoutCalls, 0, "timeout still armed after `sent`");

  h.push(frame("failure"));
  assert.equal(await h.outcome, "failed");
  assert.equal(h.cancelTimeoutCalls, 1, "timeout cancelled, not awaited");
  assert.equal(h.unsubscribeCalls, 1);
});

test("awaitLiveSwitchOutcome stays unsettled after a provisional sent, then resolves pending via the timeout when no positive terminal arrives", async () => {
  // Busy-path success now emits a positive `switched` terminal when the
  // requeued session applies the model — but a busy turn can outlast the
  // fallback timeout. If no terminal arrives in time, the pick resolves
  // `"pending"` (accepted, apply deferred), NEVER a false `"ok"`.
  const h = harness([CH_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("sent"));
  await drainMicrotasks();
  assert.equal(settled, false, "provisional `sent` must not resolve the pick");
  assert.equal(h.cancelTimeoutCalls, 0, "timeout still armed after `sent`");

  h.fireTimeout();
  assert.equal(await h.outcome, "pending");
  assert.equal(h.unsubscribeCalls, 1, "timeout fallback unsubscribes");
});

test("awaitLiveSwitchOutcome resolves ok when the busy-path deferred apply emits a positive switched terminal", async () => {
  // The K1 mirror case: after the provisional `sent`, the requeued session
  // applies the model and the harness emits a real `switched` terminal before
  // the timeout. That positive frame — not timeout silence — resolves `"ok"`.
  const h = harness([CH_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("sent"));
  await drainMicrotasks();
  assert.equal(settled, false, "provisional `sent` must not resolve the pick");

  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
  assert.equal(
    h.cancelTimeoutCalls,
    1,
    "positive terminal cancels the timeout",
  );
  assert.equal(h.unsubscribeCalls, 1);
});

test("awaitLiveSwitchOutcome never resolves ok when a delayed rejection lands after the timeout already resolved pending", async () => {
  // The K1 named pin: a busy switch whose turn outlasts the timeout. The pick
  // resolves `"pending"` at the timeout; the deferred apply then rejects. The
  // late `failure` frame must not re-resolve, and the outcome is never `"ok"`.
  const h = harness([CH_A]);

  h.push(frame("sent"));
  h.fireTimeout();
  assert.equal(await h.outcome, "pending");
  assert.equal(h.unsubscribeCalls, 1, "timeout fallback detaches the listener");

  // Deferred apply rejects after the fact: inert, the listener is gone.
  h.push(frame("failure"));
  assert.equal(
    h.unsubscribeCalls,
    1,
    "no re-resolve or re-unsubscribe on a late frame",
  );
});

test("awaitLiveSwitchOutcome ignores frames for a different request id or control type", async () => {
  const h = harness([CH_A]);
  // A replayed terminal frame from an EARLIER pick carries a different
  // requestId; it must not advance this pick's count.
  h.push(frame("switched", { requestId: "req-stale" }));
  h.push({
    type: "cancel_turn",
    status: "sent",
    requestId: REQUEST_ID,
    channelId: CH_A,
  });
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });
  await drainMicrotasks();
  assert.equal(settled, false, "unrelated frames must not advance the count");

  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome resolves pending via the timeout fallback when the harness never replies", async () => {
  const h = harness([CH_A, CH_B]);
  h.fireTimeout();
  assert.equal(await h.outcome, "pending");
  assert.equal(h.unsubscribeCalls, 1, "timeout fallback unsubscribes");
});

test("awaitLiveSwitchOutcome fires the per-channel sends after subscribing", async () => {
  const h = harness([CH_A]);
  // The subscription is registered before the sends fire, so a frame arriving
  // mid-send is never dropped. Awaiting sendStarted proves sends ran. Uses a
  // terminal-success frame (`switched`) since the provisional `sent` no longer
  // settles the pick on its own.
  await h.sendStarted;
  h.push(frame("switched"));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome with zero channels resolves pending at the timeout (no acks expected)", async () => {
  // No active turns means an empty channel set: the success resolve only fires
  // inside a frame callback keyed on an expected channel, so with no frames the
  // timeout fallback is what settles it — to `"pending"`, since no positive
  // terminal confirmed. This documents the degenerate path.
  const h = harness([]);
  h.fireTimeout();
  assert.equal(await h.outcome, "pending");
});

test("awaitLiveSwitchOutcome ignores a reconnect replay of an identical terminal frame", async () => {
  // The observer relay requests a five-minute replay on reconnect, so the SAME
  // terminal frame for one channel can arrive twice. A scalar count would treat
  // the replay as a second channel's ack and settle a two-channel pick early.
  const h = harness([CH_A, CH_B]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("switched", { channelId: CH_A }));
  h.push(frame("switched", { channelId: CH_A })); // replay of the same frame
  await drainMicrotasks();
  assert.equal(
    settled,
    false,
    "a duplicated channel-A success must not stand in for channel B",
  );

  h.push(frame("switched", { channelId: CH_B }));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome does not let one channel's duplicated success mask another channel's later failure", async () => {
  // Two channels: A succeeds and its frame is replayed; B rejects late. A
  // per-frame count would resolve "ok" on A's duplicate before B's failure and
  // report a false success. Counting per distinct channel keeps the pick open
  // for B, which fail-fasts to "failed".
  const h = harness([CH_A, CH_B]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("switched", { channelId: CH_A }));
  h.push(frame("switched", { channelId: CH_A })); // duplicate for A
  await drainMicrotasks();
  assert.equal(settled, false, "A's duplicate must not complete the pick");

  h.push(frame("failure", { channelId: CH_B }));
  assert.equal(await h.outcome, "failed");
});

test("awaitLiveSwitchOutcome ignores a stale result from an overlapping same-model operation", async () => {
  // Two picks for the same model overlap: this operation is `req-new`; a prior
  // `req-old` pick's terminal frame (same model, same channel) is still in
  // flight. Correlating on requestId — not modelId — keeps the old result from
  // settling the new pick.
  const h = harness([CH_A], "req-new");
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push({
    type: "switch_model",
    status: "switched",
    requestId: "req-old",
    channelId: CH_A,
  });
  await drainMicrotasks();
  assert.equal(settled, false, "a prior same-model pick's result is inert");

  h.push({
    type: "switch_model",
    status: "switched",
    requestId: "req-new",
    channelId: CH_A,
  });
  assert.equal(await h.outcome, "ok");
});

// The channel guard must run BEFORE status handling so a negative frame is
// correlated by channel too — a misrouted or channel-less `failure`/
// `unsupported_model` carrying this pick's requestId must not fail it. This is
// the false-failure mirror of the false-success class the positive-terminal
// channel count already guards.
test("awaitLiveSwitchOutcome ignores a failure frame from a foreign channel", async () => {
  const h = harness([CH_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  // Same requestId, but a channel this pick never fired to: inert.
  h.push(frame("failure", { channelId: "channel-foreign" }));
  await drainMicrotasks();
  assert.equal(
    settled,
    false,
    "a foreign-channel failure must not fail the pick",
  );

  h.push(frame("switched", { channelId: CH_A }));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome ignores an unsupported_model frame from a foreign channel", async () => {
  const h = harness([CH_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("unsupported_model", { channelId: "channel-foreign" }));
  await drainMicrotasks();
  assert.equal(
    settled,
    false,
    "a foreign-channel unsupported_model must not fail the pick",
  );

  h.push(frame("switched", { channelId: CH_A }));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome ignores a failure frame that carries no channel", async () => {
  const h = harness([CH_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("failure", { channelId: undefined }));
  await drainMicrotasks();
  assert.equal(settled, false, "a channel-less failure must not fail the pick");

  h.push(frame("switched", { channelId: CH_A }));
  assert.equal(await h.outcome, "ok");
});

test("awaitLiveSwitchOutcome ignores an unsupported_model frame that carries no channel", async () => {
  const h = harness([CH_A]);
  let settled = false;
  void h.outcome.then(() => {
    settled = true;
  });

  h.push(frame("unsupported_model", { channelId: undefined }));
  await drainMicrotasks();
  assert.equal(
    settled,
    false,
    "a channel-less unsupported_model must not fail the pick",
  );

  h.push(frame("switched", { channelId: CH_A }));
  assert.equal(await h.outcome, "ok");
});

import assert from "node:assert/strict";
import test from "node:test";

import {
  AGENT_RECOVERY_BACKOFF_MS,
  beginAgentRecovery,
  recordFailedRecoveryAttempt,
  recoveryAttemptDue,
  recoveryExhausted,
  recoveryLifecycleHealthy,
} from "./agentRecoveryPolicy.ts";

test("recovery uses bounded 5s, 30s, 2m backoff", () => {
  let state = beginAgentRecovery(1_000, "crash");
  assert.equal(state.nextAttemptAt, 6_000);
  assert.equal(recoveryAttemptDue(state, 5_999, false), false);
  assert.equal(recoveryAttemptDue(state, 6_000, true), false);
  assert.equal(recoveryAttemptDue(state, 6_000, false), true);

  state = recordFailedRecoveryAttempt(state, 6_000, "retry 1");
  assert.equal(state.nextAttemptAt, 6_000 + AGENT_RECOVERY_BACKOFF_MS[1]);
  state = recordFailedRecoveryAttempt(state, state.nextAttemptAt, "retry 2");
  assert.equal(state.nextAttemptAt, 36_000 + AGENT_RECOVERY_BACKOFF_MS[2]);
  state = recordFailedRecoveryAttempt(state, state.nextAttemptAt, "retry 3");
  assert.equal(recoveryExhausted(state), true);
  assert.equal(state.nextAttemptAt, Number.POSITIVE_INFINITY);
});

test("recovery is reported only after a listener lifecycle is healthy", () => {
  assert.equal(recoveryLifecycleHealthy("starting"), false);
  assert.equal(recoveryLifecycleHealthy("failed"), false);
  assert.equal(recoveryLifecycleHealthy("stopped"), false);
  assert.equal(recoveryLifecycleHealthy("listening"), true);
  assert.equal(recoveryLifecycleHealthy("waking"), true);
  assert.equal(recoveryLifecycleHealthy("ready"), true);
});

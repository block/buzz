import assert from "node:assert/strict";
import test from "node:test";
import { ACK_TIMEOUT_MS, FINISH_TIMEOUT_MS, jobPhase, phaseText } from "./browserCommandJob.ts";

test("job phases: sending -> waiting -> accepted -> done, with hard timeouts and failures", () => {
  const base = { sentAtMs: null, acceptedAtMs: null, result: null, nowMs: 1000 };
  assert.equal(jobPhase(base), "sending");
  assert.equal(jobPhase({ ...base, sentAtMs: 1000 }), "waiting");
  assert.equal(jobPhase({ ...base, sentAtMs: 1000, nowMs: 1000 + ACK_TIMEOUT_MS + 1 }), "timeout");
  assert.equal(jobPhase({ ...base, sentAtMs: 1000, result: "accepted", acceptedAtMs: 2000, nowMs: 2000 + ACK_TIMEOUT_MS + 5 }), "accepted");
  assert.equal(jobPhase({ ...base, sentAtMs: 1000, result: "accepted", acceptedAtMs: 2000, nowMs: 2000 + FINISH_TIMEOUT_MS + 1 }), "timeout");
  assert.equal(jobPhase({ ...base, sentAtMs: 1000, result: "ok" }), "done");
  assert.equal(jobPhase({ ...base, sentAtMs: 1000, result: "error" }), "failed");
  assert.match(phaseText("timeout", "Add x"), /didn't respond.*Allow remote agent management.*Reload the desktop app/);
  assert.match(phaseText("failed", "Add x", "Too many"), /failed — Too many/);
});
